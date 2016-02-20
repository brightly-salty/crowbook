// Copyright (C) 2016 Élisabeth HENRY.
//
// This file is part of Crowbook.
//
// Crowbook is free software: you can redistribute it and/or modify
// it under the terms of the GNU Lesser General Public License as published
// by the Free Software Foundation, either version 2.1 of the License, or
// (at your option) any later version.
//
// Caribon is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU Lesser General Public License for more details.
//
// You should have received ba copy of the GNU Lesser General Public License
// along with Crowbook.  If not, see <http://www.gnu.org/licenses/>.

use error::{Error,Result};
use token::Token;
use html::HtmlRenderer;
use book::{Book,Number};
use zipper::Zipper;
use templates::epub::*;
use templates::epub3;

use mustache;
use chrono;
use uuid;

use std::io::{Read,Write};
use std::path::Path;
use std::fs::File;

/// Renderer for Epub
///
/// Uses part of the HTML renderer
pub struct EpubRenderer<'a> {
    book: &'a Book,
    current_numbering: bool,
    current_chapter: i32,
    toc: Vec<String>,
    html: HtmlRenderer<'a>,
}

impl<'a> EpubRenderer<'a> {
    /// Creates a new Epub renderer
    pub fn new(book: &'a Book) -> EpubRenderer<'a> {
         EpubRenderer {
            book: book,
            html: HtmlRenderer::new(book),
            current_numbering: book.numbering,
            current_chapter: 1,
            toc: vec!(),
        }
    }

    /// Render a book
    pub fn render_book(&mut self) -> Result<String> {
        let mut zipper = try!(Zipper::new(&self.book.temp_dir));
        
        // Write mimetype
        try!(zipper.write("mimetype", b"application/epub+zip"));

        // Write chapters        
        for (i, &(n, ref v)) in self.book.chapters.iter().enumerate() {
            match n {
                Number::Unnumbered => self.current_numbering = false,
                Number::Default => self.current_numbering = self.book.numbering,
                Number::Specified(n) => {
                    self.current_numbering = self.book.numbering;
                    self.current_chapter = n;
                }
            }
            let chapter = try!(self.render_chapter(v));

            try!(zipper.write(&filenamer(i), &chapter.as_bytes()));
        }
        
        // Write CSS file
        try!(zipper.write("stylesheet.css",
                          &try!(self.book.get_template("epub_css")).as_bytes()));

        // Write titlepage
        try!(zipper.write("title_page.xhtml", &try!(self.render_titlepage()).as_bytes()));

        // Write file for ibook (why?)
        try!(zipper.write("META-INF/com.apple.ibooks.display-options.xml", IBOOK.as_bytes()));

        // Write container.xml
        try!(zipper.write("META-INF/container.xml", CONTAINER.as_bytes()));

        // Write nav.xhtml
        try!(zipper.write("nav.xhtml", &try!(self.render_nav()).as_bytes()));

        // Write content.opf
        try!(zipper.write("content.opf", &try!(self.render_opf()).as_bytes()));

        // Write toc.ncx
        try!(zipper.write("toc.ncx", &try!(self.render_toc()).as_bytes()));

        // Write the cover (if needs be)
        if let Some(ref cover) = self.book.cover {
            let s: &str = &*cover;
            let mut f = try!(File::open(s).map_err(|_| Error::FileNotFound(String::from(s))));
            let mut content = vec!();
            try!(f.read_to_end(&mut content).map_err(|_| Error::Render("Error while reading cover file")));
            try!(zipper.write(s, &content));

            // also write cover.xhtml
            try!(zipper.write("cover.xhtml", &try!(self.render_cover()).as_bytes()));
        }

        if let Some(ref epub_file) = self.book.output_epub {
            let res = try!(zipper.generate_epub(epub_file));
            Ok(res)
        } else {
            Err(Error::Render("no output epub file specified in book config"))
        }
    }
    
    /// Render the titlepgae
    fn render_titlepage(&self) -> Result<String> {
        let template = mustache::compile_str(if self.book.epub_version == 3 {epub3::TITLE} else {TITLE});
        let data = self.book.get_mapbuilder()
            .build();
        let mut res:Vec<u8> = vec!();
        template.render_data(&mut res, &data);
        match String::from_utf8(res) {
            Err(_) => Err(Error::Render("generated HTML in titlepage was not utf-8 valid")),
            Ok(res) => Ok(res)
        }
    }
    
    /// Render toc.ncx
    fn render_toc(&self) -> Result<String> {
        let mut nav_points = String::new();

        for (n, ref title) in self.toc.iter().enumerate() {
            let filename = filenamer(n);
            let id = format!("navPoint-{}", n + 1);
            nav_points.push_str(&format!(
"   <navPoint id=\"{}\">
      <navLabel>
        <text>{}</text>
      </navLabel>
      <content src = \"{}\" />
    </navPoint>\n", id, title, filename));
        }
        let template = mustache::compile_str(TOC);
        let data = self.book.get_mapbuilder()
            .insert_str("nav_points", nav_points)
            .build();
        let mut res:Vec<u8> = vec!();
        template.render_data(&mut res, &data);
        match String::from_utf8(res) {
            Err(_) => Err(Error::Render("generated HTML in toc.ncx was not valid utf-8")),
            Ok(res) => Ok(res)
        }
    }

    /// Render content.opf
    fn render_opf(&self) -> Result<String> {
        // Optional metadata
        let mut cover_xhtml = String::new();
        let mut optional = String::new();
        if let Some(ref s) = self.book.description {
            optional.push_str(&format!("<dc:description>{}</dc:description>\n", s));
        }
        if let Some(ref s) = self.book.subject {
            optional.push_str(&format!("<dc:subject>{}</dc:subject>\n", s));
        }
        if let Some(ref s) = self.book.cover {
            optional.push_str(&format!("<meta name = \"cover\" content = \"{}\" />\n", s));
            cover_xhtml.push_str(&format!("<reference type=\"cover\" title=\"Cover\" href=\"cover.xhtml\" />"));
        }

        // date
        let date = chrono::UTC::now().format("%Y-%m-%dT%H:%M:%SZ");

        // uuid
        let uuid = uuid::Uuid::new_v4().to_urn_string();
        
        let mut items = String::new();
        let mut itemrefs = String::new();
        let mut coverref = String::new();
        if let Some(_) = self.book.cover {
            items.push_str("<item id = \"cover_xhtml\" href = \"cover.xhtml\" media-type = \"application/xhtml+xml\" />\n");
            coverref.push_str("<itemref idref = \"cover_xhtml\" />");
        }
        for n in 0..self.toc.len() {
            let filename = filenamer(n);
            items.push_str(&format!("<item id = \"{}\" href = \"{}\" media-type=\"application/xhtml+xml\" />\n",
                                    to_id(&filename),
                                    filename));
            itemrefs.push_str(&format!("<itemref idref=\"{}\" />\n", to_id(&filename)));
        }
        // oh we must put cover in the manifest too
        if let Some(ref s) = self.book.cover {
            let format = if let Some(ext) = Path::new(s).extension() {
                if let Some(extension) = ext.to_str() {
                    match extension {
                        "png" => "png",
                        "jpg" | "jpeg" => "jpeg",
                        "gif" => "gif",
                        _ => {
                            println!("Warning: could not guess cover format based on extension. Assuming png.");
                            "png"
                        },
                    }
                } else {
                    println!("Warning: could not guess cover format based on extension. Assuming png.");
                    "png"
                }
            } else {
                println!("Warning: could not guess cover format based on extension. Assuming png.");
                "png"
            };
            items.push_str(&format!("<item {} media-type = \"image/{}\" id =\"{}\" href = \"{}\" />\n",
                                    if self.book.epub_version == 3 { "properties=\"cover-image\"" } else { "" },
                                    format,
                                    to_id(s),
                                    s));
        }

        let template = mustache::compile_str(if self.book.epub_version == 3 {epub3::OPF} else {OPF});
        let data = self.book.get_mapbuilder()
            .insert_str("optional", optional)
            .insert_str("items", items)
            .insert_str("itemrefs", itemrefs)
            .insert_str("date", date)
            .insert_str("uuid", uuid)
            .insert_str("cover_xhtml", cover_xhtml)
            .insert_str("coverref", coverref)
            .build();
        let mut res:Vec<u8> = vec!();
        template.render_data(&mut res, &data);
        match String::from_utf8(res) {
            Err(_) => Err(Error::Render("generated HTML in content.opf was not valid utf-8")),
            Ok(res) => Ok(res)
        }
    }

    /// Render cover.xhtml
    fn render_cover(&self) -> Result<String> {
        if let Some(ref cover) = self.book.cover {
            let template = mustache::compile_str(if self.book.epub_version == 3 {epub3::COVER} else {COVER});
            let data = self.book.get_mapbuilder()
                .insert_str("cover", cover.clone())
                .build();
            let mut res:Vec<u8> = vec!();
            template.render_data(&mut res, &data);
            match String::from_utf8(res) {
                Err(_) => Err(Error::Render("generated HTML for cover.xhtml was not utf-8 valid")),
                Ok(res) => Ok(res)
            }
        } else {
            panic!("Why is this method called if cover is None???");
        }
    }

    /// Render nav.xhtml
    fn render_nav(&self) -> Result<String> {
        let mut content = String::new();
        for (x, ref title) in self.toc.iter().enumerate() {
            let link = filenamer(x);
            content.push_str(&format!("<li><a href = \"{}\">{}</a></li>\n",
                                 link,
                                 title));
        }           
        
        let template = mustache::compile_str(if self.book.epub_version == 3 {epub3::NAV} else {NAV});
        let data = self.book.get_mapbuilder()
            .insert_str("content", content)
            .build();
        let mut res:Vec<u8> = vec!();
        template.render_data(&mut res, &data);
        match String::from_utf8(res) {
            Err(_) => Err(Error::Render("generated HTML in nav.xhtml was not utf-8 valid")),
            Ok(res) => Ok(res)
        }
    }

    /// Render a chapter
    pub fn render_chapter(&mut self, v: &[Token]) -> Result<String> {
        let mut content = String::new();
        let mut title = String::new();

        for token in v {
            content.push_str(&self.parse_token(&token, &mut title));
        }
        if title.is_empty() {
            if self.current_numbering {
                self.current_chapter += 1;
                title = format!("Chapitre {}", self.current_chapter);
            } else {
                return Err(Error::Render("chapter without h1 tag is not OK if numbering is off"));
            }
        }
        self.toc.push(title.clone());

        let template = mustache::compile_str(try!(self.book.get_template("epub_template")).as_ref());
        let data = self.book.get_mapbuilder()
            .insert_str("content", content)
            .insert_str("chapter_title", title)
            .build();
        let mut res:Vec<u8> = vec!();
        template.render_data(&mut res, &data);
        match String::from_utf8(res) {
            Err(_) => Err(Error::Render("generated HTML was not utf-8 valid")),
            Ok(res) => Ok(res)
        }
    }

    fn parse_token(&mut self, token: &Token, title: &mut String) -> String {
        match *token {
            Token::Header(n, ref vec) => {
                let s = if n == 1 && self.current_numbering {
                    let chapter = self.current_chapter;
                    self.current_chapter += 1;
                    self.book.get_header(chapter, &self.html.render_vec(vec)).unwrap()
                } else {
                    self.html.render_vec(vec)
                };
                if n == 1 {
                    if title.is_empty() {
                        *title = s.clone();
                    } else {
                        println!("Warning: detected two chapters inside the same markdown file.");
                        println!("conflict between: {} and {}", title, s);
                    }
                }
                format!("<h{}>{}</h{}>\n", n, s, n)
            },
            _ => self.html.parse_token(token)
        }
    }
}

// generate an id compatible string, replacing / and . by _
fn to_id(s: &str) -> String {
    s.replace(".", "_").replace("/", "_")
}
    
/// Generate a file name given an int   
fn filenamer(i: usize) -> String {
    format!("chapter_{:03}.xhtml", i)
}
