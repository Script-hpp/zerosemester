use printpdf::*;
use printpdf::indices::{PdfPageIndex, PdfLayerIndex};
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE};
use serde_json::Value;
use std::collections::HashMap;
use std::io::BufWriter;
use std::path::{Path, PathBuf};

const UNCATEGORIZED: &str = "Uncategorized";
const UNTITLED: &str = "Untitled";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotionPage {
    pub id: String,
    pub title: String,
    pub category: String,
}

impl std::fmt::Display for NotionPage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{}] {}", self.category, self.title)
    }
}

fn extract_title(properties: &serde_json::Map<String, Value>) -> Option<String> {
    for (_key, prop_value) in properties {
        if prop_value["type"] == "title" {
            if let Some(title_array) = prop_value["title"].as_array() {
                if let Some(first_item) = title_array.first() {
                    if let Some(text) = first_item["plain_text"].as_str() {
                        return Some(text.to_string());
                    }
                }
            }
        }
    }
    None
}

fn parent_id(item: &Value) -> Option<String> {
    let parent = &item["parent"];
    match parent["type"].as_str()? {
        "page_id" => parent["page_id"].as_str().map(|s| s.to_string()),
        "database_id" => parent["database_id"].as_str().map(|s| s.to_string()),
        _ => None,
    }
}

fn relation_target(page: &Value, id_to_title: &HashMap<String, String>) -> Option<String> {
    let properties = page["properties"].as_object()?;
    for (_key, prop_value) in properties {
        if prop_value["type"] != "relation" {
            continue;
        }
        let relation_arr = prop_value["relation"].as_array()?;
        let first_relation = relation_arr.first()?;
        let rel_id = first_relation["id"].as_str()?;
        if let Some(name) = id_to_title.get(rel_id) {
            return Some(name.clone());
        }
    }
    None
}

fn tag_target(page: &Value) -> Option<String> {
    let properties = page["properties"].as_object()?;

    for (_key, prop_value) in properties {
        match prop_value["type"].as_str()? {
            "multi_select" => {
                let arr = prop_value["multi_select"].as_array()?;
                if let Some(first) = arr.first() {
                    if let Some(name) = first["name"].as_str() {
                        return Some(name.to_string());
                    }
                }
            }
            "select" => {
                if let Some(name) = prop_value["select"]["name"].as_str() {
                    return Some(name.to_string());
                }
            }
            _ => {}
        }
    }
    None
}

pub async fn fetch_pages() -> Result<Vec<NotionPage>, Box<dyn std::error::Error + Send + Sync>> {
    let token = std::fs::read_to_string("notion_token.txt")
        .map_err(|_| "No token file found")?
        .trim()
        .to_string();

    let mut headers = HeaderMap::new();
    headers.insert(AUTHORIZATION, HeaderValue::from_str(&format!("Bearer {}", token))?);
    headers.insert("Notion-Version", HeaderValue::from_static("2022-06-28"));
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

    let client = reqwest::Client::new();

    let mut all_results = Vec::new();
    let mut cursor: Option<String> = None;

    loop {
        let mut body = serde_json::json!({ "page_size": 100 });
        if let Some(c) = &cursor {
            body["start_cursor"] = serde_json::json!(c);
        }

        let res = client
            .post("https://api.notion.com/v1/search")
            .headers(headers.clone())
            .json(&body)
            .send()
            .await?;

        let json: Value = res.json().await?;

        if let Some(results) = json["results"].as_array() {
            all_results.extend(results.clone());
        }

        if json["has_more"].as_bool().unwrap_or(false) {
            cursor = json["next_cursor"].as_str().map(|s| s.to_string());
        } else {
            break;
        }
    }

    let mut id_to_title = HashMap::new();
    let mut pages = Vec::new();

    for item in &all_results {
        let id = item["id"].as_str().unwrap_or("").to_string();
        let mut title = UNTITLED.to_string();

        if item["object"] == "database" {
            if let Some(title_arr) = item["title"].as_array() {
                if let Some(first) = title_arr.first() {
                    if let Some(text) = first["plain_text"].as_str() {
                        title = text.to_string();
                    }
                }
            }
            id_to_title.insert(id.clone(), title);
        } else if item["object"] == "page" {
            if let Some(properties) = item["properties"].as_object() {
                if let Some(found) = extract_title(properties) {
                    title = found;
                }
            }
            id_to_title.insert(id.clone(), title.clone());
            pages.push((item.clone(), title));
        }
    }

    let mut categorized_pages = Vec::new();

    for (page, title) in &pages {
        let category = relation_target(page, &id_to_title)
            .or_else(|| tag_target(page))
            .or_else(|| {
                let pid = parent_id(page)?;
                id_to_title.get(&pid).cloned()
            })
            .unwrap_or_else(|| UNCATEGORIZED.to_string());

        let id = page["id"].as_str().unwrap_or("").to_string();
        categorized_pages.push(NotionPage {
            id,
            title: title.clone(),
            category,
        });
    }

    categorized_pages.sort_by(|a, b| a.category.cmp(&b.category).then_with(|| a.title.cmp(&b.title)));
    Ok(categorized_pages)
}

pub async fn fetch_page_blocks(
    client: &reqwest::Client,
    headers: &HeaderMap,
    block_id: &str,
    depth: usize,
) -> Result<Vec<Value>, Box<dyn std::error::Error + Send + Sync>> {
    if depth > 3 {
        return Ok(Vec::new());
    }

    let mut all_blocks = Vec::new();
    let mut cursor: Option<String> = None;

    loop {
        let mut url = format!(
            "https://api.notion.com/v1/blocks/{}/children?page_size=100",
            block_id
        );
        if let Some(c) = &cursor {
            url.push_str(&format!("&start_cursor={}", c));
        }

        let mut retries = 0;
        let res = loop {
            match client.get(&url).headers(headers.clone()).send().await {
                Ok(r) if r.status().as_u16() == 429 => {
                    retries += 1;
                    if retries > 6 {
                        return Err(format!("Notion API rate limit (429) exceeded after retries for block {}", block_id).into());
                    }
                    let wait_ms = r
                        .headers()
                        .get("retry-after")
                        .and_then(|h| h.to_str().ok())
                        .and_then(|s| s.parse::<u64>().ok())
                        .map(|s| s * 1000)
                        .unwrap_or(500 * (1 << retries));
                    tokio::time::sleep(std::time::Duration::from_millis(wait_ms)).await;
                }
                Ok(r) if r.status().is_server_error() => {
                    retries += 1;
                    if retries > 3 {
                        return Err(format!("Notion server error {} for block {}", r.status(), block_id).into());
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(1000 * retries)).await;
                }
                Ok(r) if !r.status().is_success() => {
                    let status = r.status();
                    let err_body = r.text().await.unwrap_or_else(|_| "Unknown error".to_string());
                    return Err(format!("Failed to fetch blocks for {}: status {}, {}", block_id, status, err_body).into());
                }
                Ok(r) => break r,
                Err(e) => {
                    retries += 1;
                    if retries > 3 {
                        return Err(e.into());
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(1000 * retries)).await;
                }
            }
        };

        let json: Value = res.json().await?;

        if let Some(results) = json["results"].as_array() {
            for mut block in results.iter().cloned() {
                let has_children = block["has_children"].as_bool().unwrap_or(false);
                if has_children {
                    if let Some(id) = block["id"].as_str() {
                        if let Ok(children) = Box::pin(fetch_page_blocks(client, headers, id, depth + 1)).await {
                            if !children.is_empty() {
                                block["children"] = serde_json::json!(children);
                            }
                        }
                    }
                }
                all_blocks.push(block);
            }
        }

        if json["has_more"].as_bool().unwrap_or(false) {
            cursor = json["next_cursor"].as_str().map(|s| s.to_string());
        } else {
            break;
        }
    }

    Ok(all_blocks)
}

pub fn sanitize_filename(name: &str) -> String {
    let sanitized: String = name
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            other => other,
        })
        .collect();
    let trimmed = sanitized.trim();
    if trimmed.is_empty() {
        "Untitled".to_string()
    } else {
        trimmed.to_string()
    }
}

pub async fn export_page_to_pdf(
    client: &reqwest::Client,
    headers: &HeaderMap,
    page: &NotionPage,
    output_dir: &Path,
    use_category_folders: bool,
) -> Result<PathBuf, Box<dyn std::error::Error + Send + Sync>> {
    let blocks = fetch_page_blocks(client, headers, &page.id, 0).await?;

    let target_dir = if use_category_folders {
        let cat_dir = output_dir.join(sanitize_filename(&page.category));
        std::fs::create_dir_all(&cat_dir)?;
        cat_dir
    } else {
        std::fs::create_dir_all(output_dir)?;
        output_dir.to_path_buf()
    };

    let pdf_path = target_dir.join(format!("{}.pdf", sanitize_filename(&page.title)));
    let page_clone = page.clone();
    let blocks_clone = blocks;
    let pdf_path_clone = pdf_path.clone();

    tokio::task::spawn_blocking(move || {
        render_blocks_to_pdf(&page_clone, &blocks_clone, &pdf_path_clone)
    })
    .await??;

    Ok(pdf_path)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum FontStyle {
    Regular,
    Bold,
    Italic,
    Code,
}

struct PdfLayoutContext {
    doc: PdfDocumentReference,
    current_page: PdfPageIndex,
    current_layer: PdfLayerIndex,
    current_y: Mm,
    font_regular: IndirectFontRef,
    font_bold: IndirectFontRef,
    font_italic: IndirectFontRef,
    font_code: IndirectFontRef,
}

impl PdfLayoutContext {
    fn ensure_space(&mut self, needed_height: Mm) {
        if self.current_y < needed_height + Mm(20.0) {
            let (new_page, new_layer) = self.doc.add_page(Mm(210.0), Mm(297.0), "Layer 1");
            self.current_page = new_page;
            self.current_layer = new_layer;
            self.current_y = Mm(277.0);
        }
    }

    fn write_line(&mut self, text: &str, style: FontStyle, font_size: f64, x: Mm, line_height: Mm) {
        self.ensure_space(line_height);
        let font = match style {
            FontStyle::Regular => self.font_regular.clone(),
            FontStyle::Bold => self.font_bold.clone(),
            FontStyle::Italic => self.font_italic.clone(),
            FontStyle::Code => self.font_code.clone(),
        };
        let layer = self.doc.get_page(self.current_page).get_layer(self.current_layer);
        layer.use_text(text, font_size, x, self.current_y, &font);
        self.current_y -= line_height;
    }
}

fn extract_rich_text(block: &Value, block_type: &str) -> String {
    let mut result = String::new();
    if let Some(arr) = block[block_type]["rich_text"].as_array() {
        for item in arr {
            if let Some(t) = item["plain_text"].as_str() {
                result.push_str(t);
            }
        }
    }
    result
}

fn wrap_text(text: &str, max_chars: usize) -> Vec<String> {
    if text.is_empty() {
        return vec![String::new()];
    }
    let mut lines = Vec::new();
    for paragraph in text.split('\n') {
        if paragraph.trim().is_empty() {
            lines.push(String::new());
            continue;
        }
        let mut current_line = String::new();
        for word in paragraph.split_whitespace() {
            if current_line.is_empty() {
                if word.len() > max_chars {
                    let mut start = 0;
                    let chars: Vec<char> = word.chars().collect();
                    while start < chars.len() {
                        let end = (start + max_chars).min(chars.len());
                        lines.push(chars[start..end].iter().collect());
                        start = end;
                    }
                } else {
                    current_line.push_str(word);
                }
            } else if current_line.len() + 1 + word.len() <= max_chars {
                current_line.push(' ');
                current_line.push_str(word);
            } else {
                lines.push(std::mem::take(&mut current_line));
                if word.len() > max_chars {
                    let mut start = 0;
                    let chars: Vec<char> = word.chars().collect();
                    while start < chars.len() {
                        let end = (start + max_chars).min(chars.len());
                        let chunk: String = chars[start..end].iter().collect();
                        if start + max_chars < chars.len() {
                            lines.push(chunk);
                        } else {
                            current_line = chunk;
                        }
                        start = end;
                    }
                } else {
                    current_line = word.to_string();
                }
            }
        }
        if !current_line.is_empty() {
            lines.push(current_line);
        }
    }
    lines
}

fn render_block_list(ctx: &mut PdfLayoutContext, blocks: &[Value], indent: usize) {
    let left_x = Mm(20.0 + (indent as f64) * 8.0);

    for block in blocks {
        let b_type = block["type"].as_str().unwrap_or("");
        match b_type {
            "paragraph" => {
                let text = extract_rich_text(block, "paragraph");
                if text.is_empty() {
                    ctx.current_y -= Mm(4.0);
                } else {
                    let lines = wrap_text(&text, 88_usize.saturating_sub(indent * 4));
                    for line in lines {
                        ctx.write_line(&line, FontStyle::Regular, 10.5, left_x, Mm(5.2));
                    }
                    ctx.current_y -= Mm(2.0);
                }
            }
            "heading_1" => {
                let text = extract_rich_text(block, "heading_1");
                ctx.current_y -= Mm(4.0);
                let lines = wrap_text(&text, 55_usize.saturating_sub(indent * 3));
                for line in lines {
                    ctx.write_line(&line, FontStyle::Bold, 16.0, left_x, Mm(7.5));
                }
                ctx.current_y -= Mm(2.0);
            }
            "heading_2" => {
                let text = extract_rich_text(block, "heading_2");
                ctx.current_y -= Mm(3.0);
                let lines = wrap_text(&text, 65_usize.saturating_sub(indent * 3));
                for line in lines {
                    ctx.write_line(&line, FontStyle::Bold, 13.5, left_x, Mm(6.5));
                }
                ctx.current_y -= Mm(1.5);
            }
            "heading_3" => {
                let text = extract_rich_text(block, "heading_3");
                ctx.current_y -= Mm(2.0);
                let lines = wrap_text(&text, 75_usize.saturating_sub(indent * 3));
                for line in lines {
                    ctx.write_line(&line, FontStyle::Bold, 11.5, left_x, Mm(5.8));
                }
                ctx.current_y -= Mm(1.0);
            }
            "bulleted_list_item" => {
                let text = extract_rich_text(block, "bulleted_list_item");
                let lines = wrap_text(&text, 82_usize.saturating_sub(indent * 4));
                for (i, line) in lines.iter().enumerate() {
                    if i == 0 {
                        ctx.write_line(&format!("•  {}", line), FontStyle::Regular, 10.5, left_x, Mm(5.2));
                    } else {
                        ctx.write_line(line, FontStyle::Regular, 10.5, left_x + Mm(5.0), Mm(5.2));
                    }
                }
            }
            "numbered_list_item" => {
                let text = extract_rich_text(block, "numbered_list_item");
                let lines = wrap_text(&text, 82_usize.saturating_sub(indent * 4));
                for (i, line) in lines.iter().enumerate() {
                    if i == 0 {
                        ctx.write_line(&format!("-  {}", line), FontStyle::Regular, 10.5, left_x, Mm(5.2));
                    } else {
                        ctx.write_line(line, FontStyle::Regular, 10.5, left_x + Mm(5.0), Mm(5.2));
                    }
                }
            }
            "to_do" => {
                let text = extract_rich_text(block, "to_do");
                let checked = block["to_do"]["checked"].as_bool().unwrap_or(false);
                let box_str = if checked { "[x]" } else { "[ ]" };
                let lines = wrap_text(&text, 82_usize.saturating_sub(indent * 4));
                for (i, line) in lines.iter().enumerate() {
                    if i == 0 {
                        ctx.write_line(&format!("{}  {}", box_str, line), FontStyle::Regular, 10.5, left_x, Mm(5.2));
                    } else {
                        ctx.write_line(line, FontStyle::Regular, 10.5, left_x + Mm(6.0), Mm(5.2));
                    }
                }
            }
            "quote" | "callout" => {
                let btype_str = if b_type == "quote" { "quote" } else { "callout" };
                let text = extract_rich_text(block, btype_str);
                let lines = wrap_text(&text, 80_usize.saturating_sub(indent * 4));
                for line in lines {
                    ctx.write_line(&format!("|  {}", line), FontStyle::Italic, 10.5, left_x + Mm(3.0), Mm(5.2));
                }
                ctx.current_y -= Mm(2.0);
            }
            "code" => {
                let text = extract_rich_text(block, "code");
                let lang = block["code"]["language"].as_str().unwrap_or("");
                if !lang.is_empty() {
                    ctx.write_line(&format!("[{}]", lang), FontStyle::Bold, 9.0, left_x + Mm(2.0), Mm(4.5));
                }
                for line in text.lines() {
                    let wrapped_lines = wrap_text(line, 80_usize.saturating_sub(indent * 4));
                    for w_line in wrapped_lines {
                        ctx.write_line(&w_line, FontStyle::Code, 9.5, left_x + Mm(4.0), Mm(4.5));
                    }
                }
                ctx.current_y -= Mm(2.0);
            }
            "divider" => {
                ctx.write_line("------------------------------------------------------------------------", FontStyle::Regular, 9.0, left_x, Mm(5.0));
                ctx.current_y -= Mm(2.0);
            }
            _ => {}
        }

        if let Some(children) = block["children"].as_array() {
            render_block_list(ctx, children, indent + 1);
        }
    }
}

fn render_blocks_to_pdf(
    page: &NotionPage,
    top_blocks: &[Value],
    pdf_path: &Path,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let (doc, page1, layer1) = PdfDocument::new(&page.title, Mm(210.0), Mm(297.0), "Layer 1");
    let font_regular = doc.add_builtin_font(BuiltinFont::Helvetica)?;
    let font_bold = doc.add_builtin_font(BuiltinFont::HelveticaBold)?;
    let font_italic = doc.add_builtin_font(BuiltinFont::HelveticaOblique)?;
    let font_code = doc.add_builtin_font(BuiltinFont::Courier)?;

    let mut ctx = PdfLayoutContext {
        doc,
        current_page: page1,
        current_layer: layer1,
        current_y: Mm(275.0),
        font_regular,
        font_bold,
        font_italic,
        font_code,
    };

    ctx.write_line(&format!("Category: {}", page.category), FontStyle::Italic, 10.0, Mm(20.0), Mm(6.0));
    ctx.current_y -= Mm(2.0);
    for line in wrap_text(&page.title, 42) {
        ctx.write_line(&line, FontStyle::Bold, 22.0, Mm(20.0), Mm(10.0));
    }
    ctx.current_y -= Mm(4.0);
    ctx.write_line("____________________________________________________________________", FontStyle::Regular, 10.0, Mm(20.0), Mm(8.0));
    ctx.current_y -= Mm(6.0);

    render_block_list(&mut ctx, top_blocks, 0);

    let file = std::fs::File::create(pdf_path)?;
    let mut writer = BufWriter::new(file);
    ctx.doc.save(&mut writer)?;
    Ok(())
}