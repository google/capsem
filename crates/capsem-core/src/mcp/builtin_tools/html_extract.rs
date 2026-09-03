//! HTML-to-text and HTML-to-markdown extraction for the built-in HTTP tools.

/// Extract visible text from HTML using scraper (html5ever).
/// Skips script, style, noscript, svg, and template elements.
/// Inserts newlines around block elements.
pub fn extract_text_from_html(html: &str) -> String {
    use scraper::Html;

    let doc = Html::parse_document(html);
    let mut output = String::new();
    let root = doc.root_element();

    // Prefer <body> if present, otherwise use the root
    let start = scraper::Selector::parse("body")
        .ok()
        .and_then(|sel| doc.select(&sel).next())
        .map(|el| el.id())
        .unwrap_or_else(|| root.id());

    extract_text_recursive_scraper(&doc, start, &mut output);
    collapse_whitespace(&output)
}

/// Convert HTML to markdown, preserving headings, links, lists, bold/italic,
/// code blocks, and blockquotes.
pub fn extract_markdown_from_html(html: &str) -> String {
    use scraper::Html;

    let doc = Html::parse_document(html);
    let mut output = String::new();
    let root = doc.root_element();

    let start = scraper::Selector::parse("body")
        .ok()
        .and_then(|sel| doc.select(&sel).next())
        .map(|el| el.id())
        .unwrap_or_else(|| root.id());

    extract_md_recursive(&doc, start, &mut output);
    collapse_whitespace(&output)
}

const SKIP_TAGS: &[&str] = &["script", "style", "noscript", "svg", "template"];
const BLOCK_TAGS: &[&str] = &[
    "p",
    "div",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "li",
    "tr",
    "br",
    "hr",
    "section",
    "article",
    "header",
    "footer",
    "nav",
    "main",
    "blockquote",
    "pre",
    "table",
    "ul",
    "ol",
    "dl",
    "dt",
    "dd",
    "figcaption",
    "figure",
    "details",
    "summary",
];

fn extract_text_recursive_scraper(doc: &scraper::Html, node_id: ego_tree::NodeId, output: &mut String) {
    let node_ref = match doc.tree.get(node_id) {
        Some(n) => n,
        None => return,
    };

    match node_ref.value() {
        scraper::Node::Text(text) => {
            output.push_str(text);
        }
        scraper::Node::Element(el) => {
            let tag = el.name.local.as_ref();
            if SKIP_TAGS.contains(&tag) {
                return;
            }
            let is_block = BLOCK_TAGS.contains(&tag);
            if is_block {
                output.push('\n');
            }
            for child in node_ref.children() {
                extract_text_recursive_scraper(doc, child.id(), output);
            }
            if is_block {
                output.push('\n');
            }
        }
        scraper::Node::Document => {
            for child in node_ref.children() {
                extract_text_recursive_scraper(doc, child.id(), output);
            }
        }
        _ => {}
    }
}

fn extract_md_recursive(doc: &scraper::Html, node_id: ego_tree::NodeId, output: &mut String) {
    let node_ref = match doc.tree.get(node_id) {
        Some(n) => n,
        None => return,
    };

    match node_ref.value() {
        scraper::Node::Text(text) => {
            output.push_str(text);
        }
        scraper::Node::Element(el) => {
            let tag = el.name.local.as_ref();
            if SKIP_TAGS.contains(&tag) {
                return;
            }

            match tag {
                "h1" => {
                    output.push_str("\n# ");
                    md_children(doc, node_ref, output);
                    output.push('\n');
                }
                "h2" => {
                    output.push_str("\n## ");
                    md_children(doc, node_ref, output);
                    output.push('\n');
                }
                "h3" => {
                    output.push_str("\n### ");
                    md_children(doc, node_ref, output);
                    output.push('\n');
                }
                "h4" => {
                    output.push_str("\n#### ");
                    md_children(doc, node_ref, output);
                    output.push('\n');
                }
                "h5" => {
                    output.push_str("\n##### ");
                    md_children(doc, node_ref, output);
                    output.push('\n');
                }
                "h6" => {
                    output.push_str("\n###### ");
                    md_children(doc, node_ref, output);
                    output.push('\n');
                }
                "a" => {
                    let href = el.attr("href").unwrap_or("");
                    output.push('[');
                    md_children(doc, node_ref, output);
                    output.push_str("](");
                    output.push_str(href);
                    output.push(')');
                }
                "strong" | "b" => {
                    output.push_str("**");
                    md_children(doc, node_ref, output);
                    output.push_str("**");
                }
                "em" | "i" => {
                    output.push('_');
                    md_children(doc, node_ref, output);
                    output.push('_');
                }
                "code" => {
                    output.push('`');
                    md_children(doc, node_ref, output);
                    output.push('`');
                }
                "pre" => {
                    output.push_str("\n```\n");
                    md_children(doc, node_ref, output);
                    output.push_str("\n```\n");
                }
                "blockquote" => {
                    output.push_str("\n> ");
                    md_children(doc, node_ref, output);
                    output.push('\n');
                }
                "li" => {
                    // Check parent to decide bullet vs number
                    if let Some(parent) = node_ref.parent() {
                        if let scraper::Node::Element(pel) = parent.value() {
                            if pel.name.local.as_ref() == "ol" {
                                // Find our index among siblings
                                let idx = parent.children()
                                    .filter(|c| matches!(c.value(), scraper::Node::Element(e) if e.name.local.as_ref() == "li"))
                                    .position(|c| c.id() == node_id)
                                    .unwrap_or(0);
                                output.push_str(&format!("\n{}. ", idx + 1));
                            } else {
                                output.push_str("\n- ");
                            }
                        } else {
                            output.push_str("\n- ");
                        }
                    } else {
                        output.push_str("\n- ");
                    }
                    md_children(doc, node_ref, output);
                }
                "br" => {
                    output.push('\n');
                }
                "hr" => {
                    output.push_str("\n---\n");
                }
                "img" => {
                    let alt = el.attr("alt").unwrap_or("");
                    if !alt.is_empty() {
                        output.push_str(&format!("[image: {alt}]"));
                    }
                }
                _ => {
                    let is_block = BLOCK_TAGS.contains(&tag);
                    if is_block {
                        output.push('\n');
                    }
                    md_children(doc, node_ref, output);
                    if is_block {
                        output.push('\n');
                    }
                }
            }
        }
        scraper::Node::Document => {
            for child in node_ref.children() {
                extract_md_recursive(doc, child.id(), output);
            }
        }
        _ => {}
    }
}

fn md_children(doc: &scraper::Html, node_ref: ego_tree::NodeRef<scraper::Node>, output: &mut String) {
    for child in node_ref.children() {
        extract_md_recursive(doc, child.id(), output);
    }
}

/// Collapse runs of whitespace and newlines into single space/newline, then trim.
pub fn collapse_whitespace(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    let mut prev_was_newline = false;
    let mut prev_was_space = false;

    for ch in input.chars() {
        if ch == '\n' {
            if !prev_was_newline {
                result.push('\n');
            }
            prev_was_newline = true;
            prev_was_space = false;
        } else if ch.is_whitespace() {
            if !prev_was_space && !prev_was_newline {
                result.push(' ');
            }
            prev_was_space = true;
        } else {
            prev_was_newline = false;
            prev_was_space = false;
            result.push(ch);
        }
    }

    result.trim().to_string()
}
