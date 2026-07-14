use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE};
use serde_json::Value;
use std::collections::HashMap;

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

// Looks for a non-empty multi_select or select property (e.g. "Type") and returns its first value.
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

pub async fn fetch_pages() -> Result<Vec<String>, Box<dyn std::error::Error>> {
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
        let mut title = "Unbenannt".to_string();

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
    let mut via_relation = 0;
    let mut via_tag = 0;
    let mut via_parent = 0;
    let mut unresolved = 0;

    for (page, title) in &pages {
        let parent_name = if let Some(name) = relation_target(page, &id_to_title) {
            via_relation += 1;
            name
        } else if let Some(name) = tag_target(page) {
            via_tag += 1;
            name
        } else if let Some(pid) = parent_id(page) {
            if let Some(name) = id_to_title.get(&pid) {
                via_parent += 1;
                name.clone()
            } else {
                unresolved += 1;
                "Allgemein".to_string()
            }
        } else {
            unresolved += 1;
            "Allgemein".to_string()
        };

        if title != "To-do" && !title.contains("Study Space") {
            categorized_pages.push(format!("[{}] {}", parent_name, title));
        }
    }

    eprintln!(
        "{} via relation, {} via tag, {} via parent container, {} fully unresolved (of {})",
        via_relation, via_tag, via_parent, unresolved, pages.len()
    );

    categorized_pages.sort();
    Ok(categorized_pages)
}