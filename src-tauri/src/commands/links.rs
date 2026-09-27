use reqwest::Client;
use scraper::{Html, Selector};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct LinkMetadata {
    pub title: Option<String>,
    pub description: Option<String>,
    pub image: Option<String>,
    pub site_name: Option<String>,
}

// NOTE: async so the request runs off the main thread. A sync command runs on
// the main thread and froze the whole UI until the page answered or timed out.
#[tauri::command]
pub async fn fetch_link_metadata(url: String) -> Result<LinkMetadata, String> {
    let client = Client::builder()
        .user_agent("Mozilla/5.0 (compatible; ClipboardManager/1.0)")
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .map_err(|e| e.to_string())?;

    let response = client.get(&url).send().await.map_err(|e| e.to_string())?;
    let html_content = response.text().await.map_err(|e| e.to_string())?;
    let document = Html::parse_document(&html_content);

    let mut metadata = LinkMetadata {
        title: None,
        description: None,
        image: None,
        site_name: None,
    };

    // Try Open Graph tags first
    if let Ok(og_title_selector) = Selector::parse("meta[property='og:title']") {
        if let Some(element) = document.select(&og_title_selector).next() {
            metadata.title = element.value().attr("content").map(|s| s.to_string());
        }
    }

    if let Ok(og_desc_selector) = Selector::parse("meta[property='og:description']") {
        if let Some(element) = document.select(&og_desc_selector).next() {
            metadata.description = element.value().attr("content").map(|s| s.to_string());
        }
    }

    if let Ok(og_image_selector) = Selector::parse("meta[property='og:image']") {
        if let Some(element) = document.select(&og_image_selector).next() {
            metadata.image = element.value().attr("content").map(|s| s.to_string());
        }
    }

    if let Ok(og_site_selector) = Selector::parse("meta[property='og:site_name']") {
        if let Some(element) = document.select(&og_site_selector).next() {
            metadata.site_name = element.value().attr("content").map(|s| s.to_string());
        }
    }

    // Fallback to regular meta tags
    if metadata.title.is_none() {
        if let Ok(title_selector) = Selector::parse("title") {
            if let Some(element) = document.select(&title_selector).next() {
                metadata.title = Some(element.text().collect::<String>());
            }
        }
    }

    if metadata.description.is_none() {
        if let Ok(desc_selector) = Selector::parse("meta[name='description']") {
            if let Some(element) = document.select(&desc_selector).next() {
                metadata.description = element.value().attr("content").map(|s| s.to_string());
            }
        }
    }

    Ok(metadata)
}
