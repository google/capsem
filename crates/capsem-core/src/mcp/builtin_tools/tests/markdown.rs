//! Markdown extraction tests for `extract_markdown_from_html`.

use super::*;

// -----------------------------------------------------------------------
// Fixture-based markdown extraction tests
// -----------------------------------------------------------------------

#[test]
fn markdown_elie_about_has_structure() {
    let html = load_fixture("elie_about.html");
    let md = extract_markdown_from_html(&html);
    // Must contain key content
    assert!(md.contains("Bursztein"), "must contain 'Bursztein'");
    assert!(md.contains("Google"), "must contain 'Google'");
    // Must have markdown headings
    assert!(md.contains("# ") || md.contains("## "), "must have markdown headings");
    // Must have markdown links
    assert!(md.contains("]("), "must have markdown links [text](url)");
    // Must NOT contain script/style content
    assert!(!md.contains("<script"), "must not contain script tags");
    assert!(!md.contains("<style"), "must not contain style tags");
}

#[test]
fn markdown_preserves_headings() {
    let html = "<h1>Title</h1><h2>Subtitle</h2><p>Body text</p>";
    let md = extract_markdown_from_html(html);
    assert!(md.contains("# Title"), "h1 -> '# Title', got: {md:?}");
    assert!(md.contains("## Subtitle"), "h2 -> '## Subtitle', got: {md:?}");
    assert!(md.contains("Body text"), "body preserved");
}

#[test]
fn markdown_preserves_links() {
    let html = r#"<a href="https://example.com">Example</a>"#;
    let md = extract_markdown_from_html(html);
    assert!(md.contains("[Example](https://example.com)"), "link preserved: {md:?}");
}

#[test]
fn markdown_preserves_bold_italic() {
    let html = "<strong>Bold</strong> and <em>Italic</em>";
    let md = extract_markdown_from_html(html);
    assert!(md.contains("**Bold**"), "bold preserved: {md:?}");
    assert!(md.contains("_Italic_"), "italic preserved: {md:?}");
}

#[test]
fn markdown_preserves_lists() {
    let html = "<ul><li>One</li><li>Two</li></ul>";
    let md = extract_markdown_from_html(html);
    assert!(md.contains("- One"), "unordered list: {md:?}");
    assert!(md.contains("- Two"), "unordered list: {md:?}");
}

#[test]
fn markdown_preserves_ordered_lists() {
    let html = "<ol><li>First</li><li>Second</li></ol>";
    let md = extract_markdown_from_html(html);
    assert!(md.contains("1. First"), "ordered list: {md:?}");
    assert!(md.contains("2. Second"), "ordered list: {md:?}");
}

#[test]
fn markdown_preserves_code() {
    let html = "<code>let x = 1;</code>";
    let md = extract_markdown_from_html(html);
    assert!(md.contains("`let x = 1;`"), "inline code: {md:?}");
}

#[test]
fn markdown_preserves_code_blocks() {
    let html = "<pre><code>fn main() {}</code></pre>";
    let md = extract_markdown_from_html(html);
    assert!(md.contains("```"), "code block fencing: {md:?}");
    assert!(md.contains("fn main()"), "code block content: {md:?}");
}

#[test]
fn markdown_preserves_blockquotes() {
    let html = "<blockquote>A wise quote</blockquote>";
    let md = extract_markdown_from_html(html);
    assert!(md.contains("> A wise quote"), "blockquote: {md:?}");
}

#[test]
fn markdown_vs_content_mode() {
    let html = r#"<h1>Title</h1><p>Text with <a href="/link">link</a> and <strong>bold</strong>.</p>"#;
    let md = extract_markdown_from_html(html);
    let text = extract_text_from_html(html);
    // Markdown has structure markers
    assert!(md.contains("# Title"), "markdown has heading marker");
    assert!(md.contains("](/link)"), "markdown has link");
    assert!(md.contains("**bold**"), "markdown has bold");
    // Plain text has no markers
    assert!(!text.contains("# "), "text has no heading markers");
    assert!(!text.contains("]("), "text has no link markers");
    assert!(!text.contains("**"), "text has no bold markers");
    // Both have the actual content
    assert!(text.contains("Title"), "text has title");
    assert!(text.contains("bold"), "text has bold word");
}

#[test]
fn markdown_wiki_turing_has_structure() {
    let html = load_fixture("wiki_turing_excerpt.html");
    let md = extract_markdown_from_html(&html);
    assert!(md.contains("Turing"), "must contain 'Turing'");
    assert!(!md.contains("<script"), "no script leakage");
    // Wikipedia articles have links
    assert!(md.contains("]("), "must have markdown links");
}
