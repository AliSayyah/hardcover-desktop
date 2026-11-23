//! Catalogue text as plain paragraphs.
//!
//! Hardcover's descriptions and bios come from many sources: some are plain
//! text with blank lines between paragraphs, some are publisher HTML
//! (`<p><b>…</b></p> <p>…<i>Title</i>…</p>`). The app draws text, not
//! markup, so [`plain_text`] takes the tags out, turns the block ones into
//! line breaks and decodes the entities.

/// `text` as plain text: paragraphs separated by a blank line, a `<br>` as a
/// line break, inline tags (`<b>`, `<i>`, `<a>`, …) dropped with their text
/// kept, and character references decoded.
///
/// A `<` that doesn't start a tag ("a < b", "<3") stays. Text with no block
/// tags keeps its own line breaks; with them, whitespace is collapsed as a
/// browser would, so the source's line breaks don't split a paragraph.
pub(crate) fn plain_text(text: &str) -> String {
    let tokens = tokenize(text);
    let html_blocks = tokens
        .iter()
        .any(|t| matches!(t, Token::Tag { name, .. } if is_block(name) || name == "br"));
    let mut out = Out::default();
    for token in tokens {
        match token {
            Token::Text(t) => {
                let t = decode_entities(t);
                if html_blocks {
                    out.push_collapsed(&t);
                } else {
                    out.push_raw(&t);
                }
            }
            Token::Tag { name, closing } => {
                if name == "br" {
                    out.line_break();
                } else if is_block(&name) {
                    out.paragraph_break();
                    if name == "li" && !closing {
                        out.push_raw("• ");
                    }
                }
            }
        }
    }
    out.finish()
}

enum Token<'a> {
    Text(&'a str),
    Tag { name: String, closing: bool },
}

/// Cuts `text` into runs of text and tags. A tag is `<` followed by a
/// letter (or `/` and a letter, or `!` for a comment) up to its `>`;
/// anything else is text.
fn tokenize(text: &str) -> Vec<Token<'_>> {
    let mut tokens = Vec::new();
    let mut rest = text;
    let mut text_start = 0;
    let mut pos = 0;
    while let Some(lt) = rest.find('<') {
        let at = pos + lt;
        let after = &text[at + 1..];
        let (closing, name_part) = match after.strip_prefix('/') {
            Some(n) => (true, n),
            None => (false, after),
        };
        let starts_tag = name_part.starts_with(|c: char| c.is_ascii_alphabetic())
            || (!closing && after.starts_with('!'));
        let end = starts_tag.then(|| after.find('>')).flatten();
        let Some(end) = end else {
            pos = at + 1;
            rest = &text[pos..];
            continue;
        };
        let comment = after.starts_with('!');
        let name: String = name_part
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric())
            .map(|c| c.to_ascii_lowercase())
            .collect();
        // `<https://…>` or `<name@example.com>` is text, not a tag.
        let ends_name = name_part[name.len()..]
            .starts_with(|c: char| c.is_whitespace() || c == '>' || c == '/');
        if !comment && !(ends_name && (is_block(&name) || is_inline(&name) || name == "br")) {
            pos = at + 1;
            rest = &text[pos..];
            continue;
        }
        if text_start < at {
            tokens.push(Token::Text(&text[text_start..at]));
        }
        if !comment {
            tokens.push(Token::Tag { name, closing });
        }
        pos = at + 1 + end + 1;
        text_start = pos;
        rest = &text[pos..];
    }
    if text_start < text.len() {
        tokens.push(Token::Text(&text[text_start..]));
    }
    tokens
}

/// Tags that start or end a paragraph.
fn is_block(name: &str) -> bool {
    matches!(
        name,
        "p" | "div"
            | "li"
            | "ul"
            | "ol"
            | "blockquote"
            | "h1"
            | "h2"
            | "h3"
            | "h4"
            | "h5"
            | "h6"
            | "hr"
            | "section"
            | "article"
            | "pre"
            | "table"
            | "tr"
    )
}

/// Tags dropped with their text kept.
fn is_inline(name: &str) -> bool {
    matches!(
        name,
        "a" | "abbr"
            | "b"
            | "big"
            | "cite"
            | "code"
            | "del"
            | "em"
            | "font"
            | "i"
            | "img"
            | "ins"
            | "mark"
            | "q"
            | "s"
            | "small"
            | "span"
            | "strike"
            | "strong"
            | "sub"
            | "sup"
            | "td"
            | "th"
            | "tt"
            | "u"
            | "wbr"
    )
}

/// The plain text being built: breaks are held until text follows, so
/// none lead, trail or stack up.
#[derive(Default)]
struct Out {
    text: String,
    /// 0 none, 1 a line break, 2 a paragraph break.
    pending_break: u8,
    pending_space: bool,
}

impl Out {
    fn line_break(&mut self) {
        self.pending_break = self.pending_break.max(1);
    }

    fn paragraph_break(&mut self) {
        self.pending_break = 2;
    }

    fn flush(&mut self) {
        if !self.text.is_empty() {
            match self.pending_break {
                0 if self.pending_space => self.text.push(' '),
                0 => {}
                1 => self.text.push('\n'),
                _ => self.text.push_str("\n\n"),
            }
        }
        self.pending_break = 0;
        self.pending_space = false;
    }

    /// Text whose whitespace is collapsed to single spaces.
    fn push_collapsed(&mut self, t: &str) {
        let mut words = t.split_whitespace().peekable();
        if t.starts_with(char::is_whitespace) {
            self.pending_space = true;
        }
        while let Some(word) = words.next() {
            self.flush();
            self.text.push_str(word);
            if words.peek().is_some() {
                self.pending_space = true;
            }
        }
        if t.ends_with(char::is_whitespace) && !t.trim().is_empty() {
            self.pending_space = true;
        }
    }

    /// Text kept as it is, line breaks included.
    fn push_raw(&mut self, t: &str) {
        if t.is_empty() {
            return;
        }
        let at_line_start = self.text.is_empty() || self.pending_break > 0;
        let t = if at_line_start { t.trim_start() } else { t };
        if t.is_empty() {
            return;
        }
        self.flush();
        self.text.push_str(t);
    }

    fn finish(self) -> String {
        self.text.trim().to_owned()
    }
}

/// `&amp;`, `&#39;`, `&#x2014;` and the named references publisher text
/// uses. An unknown or malformed one stays as it is.
fn decode_entities(text: &str) -> std::borrow::Cow<'_, str> {
    if !text.contains('&') {
        return text.into();
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        let after = &rest[amp + 1..];
        let decoded = after
            .find(';')
            .filter(|&semi| semi <= 10)
            .and_then(|semi| Some((entity(&after[..semi])?, semi)));
        match decoded {
            Some((c, semi)) => {
                out.push(c);
                rest = &after[semi + 1..];
            }
            None => {
                out.push('&');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out.into()
}

fn entity(name: &str) -> Option<char> {
    if let Some(num) = name.strip_prefix('#') {
        let code = match num.strip_prefix(['x', 'X']) {
            Some(hex) => u32::from_str_radix(hex, 16).ok()?,
            None => num.parse().ok()?,
        };
        return char::from_u32(code).filter(|c| *c != '\0');
    }
    Some(match name {
        "amp" => '&',
        "lt" => '<',
        "gt" => '>',
        "quot" => '"',
        "apos" => '\'',
        "nbsp" => '\u{a0}',
        "ndash" => '–',
        "mdash" => '—',
        "hellip" => '…',
        "lsquo" => '‘',
        "rsquo" => '’',
        "ldquo" => '“',
        "rdquo" => '”',
        "laquo" => '«',
        "raquo" => '»',
        "copy" => '©',
        "reg" => '®',
        "trade" => '™',
        "eacute" => 'é',
        "egrave" => 'è',
        "aacute" => 'á',
        "iacute" => 'í',
        "oacute" => 'ó',
        "uacute" => 'ú',
        "ntilde" => 'ñ',
        "uuml" => 'ü',
        "ouml" => 'ö',
        "auml" => 'ä',
        "ccedil" => 'ç',
        "middot" => '·',
        "bull" => '•',
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::plain_text;

    #[test]
    fn publisher_html_becomes_paragraphs() {
        let html = "<p><b>The only history of the valley - from the reporter</b></p> \
                    <p>Over five decades, <i>The Big Score</i> chronicles how they \
                    began.</p> <p>A story with big money at its core. </p>";
        assert_eq!(
            plain_text(html),
            "The only history of the valley - from the reporter\n\n\
             Over five decades, The Big Score chronicles how they began.\n\n\
             A story with big money at its core."
        );
    }

    #[test]
    fn plain_text_keeps_its_line_breaks() {
        let text = "First paragraph.\n\nSecond one,\nwith a line break.";
        assert_eq!(plain_text(text), text);
    }

    #[test]
    fn inline_tags_without_blocks_keep_line_breaks() {
        assert_eq!(
            plain_text("Read <i>Dune</i> first.\n\nThen <em>Messiah</em>."),
            "Read Dune first.\n\nThen Messiah."
        );
    }

    #[test]
    fn breaks_lists_and_source_whitespace() {
        assert_eq!(
            plain_text("<div>One<br/>two\n  lines</div>\n<ul>\n<li>a</li><li>b</li></ul>"),
            "One\ntwo lines\n\n• a\n\n• b"
        );
    }

    #[test]
    fn entities_are_decoded() {
        assert_eq!(
            plain_text("Tom &amp; Jerry&#39;s &ldquo;tale&rdquo; &#x2014; &copy;"),
            "Tom & Jerry's “tale” — ©"
        );
        assert_eq!(plain_text("&notanentity; & co"), "&notanentity; & co");
        assert_eq!(plain_text("&lt;p&gt; is a tag"), "<p> is a tag");
    }

    #[test]
    fn a_lone_angle_bracket_is_text() {
        assert_eq!(plain_text("a < b and c > d, <3"), "a < b and c > d, <3");
        assert_eq!(plain_text("unclosed <b"), "unclosed <b");
    }

    #[test]
    fn only_html_tags_are_markup() {
        let text = "Write to <name@example.com> or see <https://author.example>.";
        assert_eq!(plain_text(text), text);
        assert_eq!(plain_text("A <blink>B</blink>"), "A <blink>B</blink>");
        assert_eq!(plain_text("<P>Upper</P><BR>case"), "Upper\n\ncase");
        assert_eq!(plain_text("One<BR/>two"), "One\ntwo");
    }

    #[test]
    fn comments_and_attributes_go() {
        assert_eq!(
            plain_text(r#"<!-- x --><p class="x">Hi <a href="https://x">there</a></p>"#),
            "Hi there"
        );
    }

    #[test]
    fn persian_text_is_untouched() {
        let text = "کتابی دربارهٔ «زندگی».\n\nبخش دوم.";
        assert_eq!(plain_text(text), text);
        assert_eq!(
            plain_text("<p>کتابی دربارهٔ <i>زندگی</i>.</p><p>بخش دوم.</p>"),
            "کتابی دربارهٔ زندگی.\n\nبخش دوم."
        );
    }
}
