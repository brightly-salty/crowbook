use crate::book::{compile_str, Book};
use crate::book_renderer::BookRenderer;
use crate::error::Result;
use crate::number::Number;
use crate::parser::Parser;
use crate::templates::odt;
use crate::token::Token;
use crate::zipper::Zipper;

use crowbook_text_processing::escape;

use std::io::Write;

/// Rendererer for ODT
///
/// Still very experimental.
pub struct OdtRenderer<'a> {
    book: &'a Book,
    current_numbering: i32,
    current_hide: bool,
    current_chapter: i32,
    automatic_styles: String,
}

impl<'a> OdtRenderer<'a> {
    /// Creates a new OdtRenderer
    pub fn new(book: &'a Book) -> OdtRenderer {
        OdtRenderer {
            book,
            current_chapter: 1,
            current_numbering: book.options.get_i32("rendering.num_depth").unwrap(),
            current_hide: false,
            automatic_styles: String::from(
                "
<style:style style:name=\"T1\" \
                                            style:family=\"text\">
  <style:text-properties \
                                            fo:font-style=\"italic\" \
                                            style:font-style-asian=\"italic\" \
                                            style:font-style-complex=\"italic\"/>
</style:style>
\
                                            <style:style style:name=\"T2\" \
                                            style:family=\"text\">
  <style:text-properties \
                                            fo:font-weight=\"bold\" \
                                            style:font-weight-asian=\"bold\" \
                                            style:font-weight-complex=\"bold\"/>
</style:style>",
            ),
        }
    }

    /// Renders a full book
    ///
    /// This will try to generate an ODT file according to self.book options.
    ///
    /// # Returns
    /// * `Ok(s)` where `s` contains the output of the `zip` command
    ///   used to create the ODT file.
    /// * An error if there was somel problem during either the rendering to
    ///   ODT format, or the generation of the ODT file itself.
    pub fn render_book(&mut self, to: &mut dyn Write) -> Result<String> {
        let content = self.render_content()?;

        let mut zipper = Zipper::new(&self.book.options.get_path("crowbook.temp_dir").unwrap())?;

        // Write template.odt there
        zipper.write("template.odt", odt::ODT, false)?;
        // unzip it
        zipper.unzip("template.odt")?;
        // Complete it with content.xml
        zipper.write("content.xml", content.as_bytes(), false)?;
        // Zip and copy
        zipper.generate_odt(
            self.book.options.get_str("crowbook.zip.command").unwrap(),
            to,
        )
    }

    /// Render content.xml
    fn render_content(&mut self) -> Result<String> {
        // Print a warning for the features that aren't supported in ODT.
        let mut missing = vec![];
        if self.book.features.image {
            missing.push(lformat!("images"));
        }
        if self.book.features.blockquote {
            missing.push(lformat!("blockquotes"));
        }
        if self.book.features.codeblock {
            missing.push(lformat!("codeblocks"));
        }
        if self.book.features.ordered_list {
            missing.push(lformat!("ordered lists"));
        }
        if self.book.features.footnote {
            missing.push(lformat!("footnotes"));
        }
        if self.book.features.table {
            missing.push(lformat!("tables"));
        }
        if self.book.features.superscript {
            missing.push(lformat!("superscript"));
        }
        if self.book.features.subscript {
            missing.push(lformat!("subscript"));
        }

        if !missing.is_empty() {
            let missing = missing.join(", ");
            warn!("{}", lformat!("ODT: The document uses the following features, that are not implemented for ODT output: {features}. They will be ignored in the generated document.",
                                 features = missing));
        }

        let mut content = String::new();

        for chapter in &self.book.chapters {
            let n = chapter.number;
            let v = &chapter.content;
            self.current_hide = false;
            match n {
                Number::Unnumbered | Number::UnnumberedPart => self.current_numbering = 0,
                Number::Default | Number::DefaultPart => {
                    self.current_numbering =
                        self.book.options.get_i32("rendering.num_depth").unwrap()
                }
                Number::Specified(n) | Number::SpecifiedPart(n) => {
                    self.current_numbering = self.book.options.get_i32("numbering").unwrap();
                    self.current_chapter = n;
                }
                Number::Hidden => {
                    self.current_numbering = 0;
                    self.current_hide = true;
                }
            }
            if n.is_part() {
                error!("{}", lformat!("Parts are not supported yet in ODT"));
            }

            for token in v {
                content.push_str(&self.parse_token(token));
            }
        }

        let template = compile_str(
            odt::CONTENT,
            &self.book.source,
            "could not compile template for content.xml",
        )?;
        let data = self
            .book
            .get_metadata(|s| Ok(s.to_owned()))?
            .insert_str("content", content)
            .insert_str("automatic_styles", self.automatic_styles.clone())
            .build();

        let mut res: Vec<u8> = vec![];
        template.render_data(&mut res, &data)?;
        match String::from_utf8(res) {
            Err(_) => panic!("{}", lformat!("generated content.xml was not utf-8 valid")),
            Ok(res) => Ok(res),
        }
    }

    /// Transform a vector of `Token`s to Odt format
    fn render_vec(&mut self, tokens: &[Token]) -> String {
        let mut res = String::new();

        for token in tokens {
            res.push_str(&self.parse_token(token));
        }
        res
    }

    fn parse_token(&mut self, token: &Token) -> String {
        match *token {
            Token::Str(ref text) => escape::html(self.book.clean(text.as_str())).into_owned(),
            Token::Paragraph(ref vec) => {
                format!(
                    "<text:p text:style-name=\"Text_20_body\">{}</text:p>\n",
                    self.render_vec(vec)
                )
            }
            Token::Header(n, ref vec) => {
                if n == 1 && self.current_hide {
                    return String::new();
                }
                let s = if n == 1 && self.current_numbering >= 1 {
                    let chapter = self.current_chapter;
                    self.current_chapter += 1;
                    let res = self
                        .book
                        .get_chapter_header(chapter, self.render_vec(vec), |s| {
                            Ok(self.render_vec(&Parser::new().parse_inline(s)?))
                        });
                    res.unwrap().text
                } else {
                    self.render_vec(vec)
                };
                format!(
                    "<text:h text:style-name=\"Heading_20_{}\">\n{}</text:h>\n",
                    n, s
                )
            }
            Token::Emphasis(ref vec) => {
                format!(
                    "<text:span text:style-name=\"T1\">{}</text:span>",
                    self.render_vec(vec)
                )
            }
            Token::Strong(ref vec) => {
                format!(
                    "<text:span text:style-name=\"T2\">{}</text:span>",
                    self.render_vec(vec)
                )
            }
            Token::List(ref vec) => format!("<text:list>\n{}</text:list>\n", self.render_vec(vec)),
            Token::OrderedList(_, ref vec) => {
                format!("<text:list>\n{}</text:list>\n", self.render_vec(vec))
            }
            Token::Item(ref vec) => {
                format!(
                    "<text:list-item>\n<text:p>{}</text:p></text:list-item>",
                    self.render_vec(vec)
                )
            }
            Token::Link(ref url, _, ref vec) => {
                format!(
                    "<text:a xlink:type=\"simple\"  xlink:href=\"{}\">{}</text:a>",
                    url,
                    self.render_vec(vec)
                )
            }
            Token::Code(ref s) => {
                format!(
                    "<text:span text:style-name=\"Preformatted_20_Text\">{}</text:span>",
                    s
                )
            }
            Token::Subscript(ref vec) | Token::Superscript(ref vec) => self.render_vec(vec),
            Token::BlockQuote(ref vec) => format!(
                "<text:p text:style-name=\"Text_20_Body\">{}</text:p>\n",
                self.render_vec(vec)
            ),
            Token::CodeBlock(_, ref s) => {
                format!("<text:p text:style-name=\"Text_20_Body\">{}</text:p>\n", s)
            }
            Token::SoftBreak | Token::HardBreak => String::from(" "),
            Token::Rule => String::from("<text:p /><text:p>***</text:p><text:p />"),
            Token::Image(_, _, _) | Token::StandaloneImage(_, _, _) => String::from(" "),
            Token::Table(_, _) | Token::TableHead(_) | Token::TableRow(_) | Token::TableCell(_) => {
                String::from(" ")
            }
            Token::FootnoteReference(..) | Token::FootnoteDefinition(..) => String::new(),
            Token::Annotation(_, ref vec) => self.render_vec(vec),
            Token::DescriptionList(ref v)
            | Token::DescriptionItem(ref v)
            | Token::DescriptionTerm(ref v)
            | Token::DescriptionDetails(ref v)
            | Token::Strikethrough(ref v)
            | Token::TaskItem(_, ref v) => {
                warn!(
                    "{}",
                    lformat!("ODT: Description list and strikethrough not handled in this output")
                );
                self.render_vec(v)
            }
        }
    }
}

pub struct Odt {}

impl BookRenderer for Odt {
    fn auto_path(&self, book_name: &str) -> Result<String> {
        Ok(format!("{}.odt", book_name))
    }

    fn render(&self, book: &Book, to: &mut dyn Write) -> Result<()> {
        OdtRenderer::new(book).render_book(to)?;
        Ok(())
    }
}
