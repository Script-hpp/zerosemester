use reqwest::{
    header::{HeaderMap, HeaderValue, AUTHORIZATION},
    multipart::Form,
    Client,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaperlessConfig {
    pub url: String,
    pub token: String,
    pub auto_create_tags: bool,
    pub add_base_tag: bool,
    pub base_tag_name: String,
}

impl Default for PaperlessConfig {
    fn default() -> Self {
        Self {
            url: "http://localhost:8000".to_string(),
            token: String::new(),
            auto_create_tags: true,
            add_base_tag: true,
            base_tag_name: "Notion".to_string(),
        }
    }
}

impl PaperlessConfig {
    pub fn load() -> Self {
        if let Ok(data) = std::fs::read_to_string("paperless_config.json") {
            if let Ok(config) = serde_json::from_str(&data) {
                return config;
            }
        }
        Self::default()
    }

    pub fn save(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let data = serde_json::to_string_pretty(self)?;
        std::fs::write("paperless_config.json", data)?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaperlessTag {
    pub id: u64,
    pub name: String,
}

#[derive(Debug, Clone)]
pub struct PaperlessDocument {
    pub id: u64,
    pub title: String,
    pub original_file_name: Option<String>,
}

#[derive(Debug, Clone)]
pub struct PaperlessMetadata {
    pub documents: Vec<PaperlessDocument>,
    pub tags: Vec<PaperlessTag>,
}

pub fn build_headers(token: &str) -> Result<HeaderMap, Box<dyn std::error::Error + Send + Sync>> {
    let mut headers = HeaderMap::new();
    let auth_val = format!("Token {}", token.trim());
    headers.insert(AUTHORIZATION, HeaderValue::from_str(&auth_val)?);
    Ok(headers)
}

pub async fn fetch_metadata(
    client: &Client,
    config: &PaperlessConfig,
) -> Result<PaperlessMetadata, Box<dyn std::error::Error + Send + Sync>> {
    let headers = build_headers(&config.token)?;
    let base_url = config.url.trim_end_matches('/');

    // 1. Fetch tags
    let mut tags = Vec::new();
    let mut next_url = Some(format!("{}/api/tags/?page_size=1000", base_url));
    while let Some(url) = next_url {
        let res = client.get(&url).headers(headers.clone()).send().await?;
        if !res.status().is_success() {
            let status = res.status();
            let body = res.text().await.unwrap_or_default();
            return Err(format!("Failed to fetch Paperless tags (HTTP {}): {}", status, body).into());
        }
        let json: Value = res.json().await?;
        if let Some(results) = json["results"].as_array() {
            for item in results {
                if let (Some(id), Some(name)) = (item["id"].as_u64(), item["name"].as_str()) {
                    tags.push(PaperlessTag {
                        id,
                        name: name.to_string(),
                    });
                }
            }
        }
        next_url = json["next"].as_str().map(|s| s.to_string());
    }

    // 2. Fetch documents (only id, title, original_file_name needed for deduplication check)
    let mut documents = Vec::new();
    let mut next_url = Some(format!("{}/api/documents/?page_size=1000", base_url));
    while let Some(url) = next_url {
        let res = client.get(&url).headers(headers.clone()).send().await?;
        if !res.status().is_success() {
            let status = res.status();
            let body = res.text().await.unwrap_or_default();
            return Err(format!("Failed to fetch Paperless documents (HTTP {}): {}", status, body).into());
        }
        let json: Value = res.json().await?;
        if let Some(results) = json["results"].as_array() {
            for item in results {
                if let (Some(id), Some(title)) = (item["id"].as_u64(), item["title"].as_str()) {
                    let original_file_name = item["original_file_name"].as_str().map(|s| s.to_string());
                    documents.push(PaperlessDocument {
                        id,
                        title: title.to_string(),
                        original_file_name,
                    });
                }
            }
        }
        next_url = json["next"].as_str().map(|s| s.to_string());
    }

    Ok(PaperlessMetadata { documents, tags })
}

pub async fn ensure_tag(
    client: &Client,
    config: &PaperlessConfig,
    headers: &HeaderMap,
    tag_name: &str,
    existing_tags: &mut Vec<PaperlessTag>,
) -> Result<u64, Box<dyn std::error::Error + Send + Sync>> {
    let clean_name = tag_name.trim();
    if clean_name.is_empty() {
        return Err("Tag name is empty".into());
    }
    // Case-insensitive match against existing tags
    if let Some(t) = existing_tags.iter().find(|t| t.name.eq_ignore_ascii_case(clean_name)) {
        return Ok(t.id);
    }

    // If tag does not exist, create new tag in Paperless
    let url = format!("{}/api/tags/", config.url.trim_end_matches('/'));
    let body = serde_json::json!({ "name": clean_name });
    let res = client.post(&url).headers(headers.clone()).json(&body).send().await?;
    if !res.status().is_success() {
        let status = res.status();
        let err_body = res.text().await.unwrap_or_default();
        return Err(format!("Failed to create tag '{}' in Paperless (HTTP {}): {}", clean_name, status, err_body).into());
    }
    let json: Value = res.json().await?;
    let new_id = json["id"]
        .as_u64()
        .ok_or_else(|| format!("No tag id returned when creating {}", clean_name))?;
    existing_tags.push(PaperlessTag {
        id: new_id,
        name: clean_name.to_string(),
    });
    Ok(new_id)
}

pub async fn upload_document(
    client: &Client,
    config: &PaperlessConfig,
    headers: &HeaderMap,
    pdf_path: &Path,
    title: &str,
    tag_ids: &[u64],
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let url = format!("{}/api/documents/post_document/", config.url.trim_end_matches('/'));

    let file_bytes = tokio::fs::read(pdf_path).await?;
    let file_name = pdf_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("document.pdf")
        .to_string();
    let part = reqwest::multipart::Part::bytes(file_bytes)
        .file_name(file_name)
        .mime_str("application/pdf")?;

    let mut form = Form::new()
        .part("document", part)
        .text("title", title.to_string());

    for tag_id in tag_ids {
        form = form.text("tags", tag_id.to_string());
    }

    let res = client.post(&url).headers(headers.clone()).multipart(form).send().await?;
    if !res.status().is_success() {
        let status = res.status();
        let body = res.text().await.unwrap_or_default();
        return Err(format!("Failed to upload document '{}' (HTTP {}): {}", title, status, body).into());
    }

    Ok(())
}
