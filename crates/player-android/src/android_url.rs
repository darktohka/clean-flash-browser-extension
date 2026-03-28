//! Android URL provider — static URL values set at launch.

use player_ui_traits::UrlProvider;

pub struct AndroidUrlProvider {
    /// The URL of the SWF being played.
    swf_url: String,
    /// The page URL (for standalone, this is the SWF URL itself).
    page_url: String,
}

impl AndroidUrlProvider {
    pub fn new(swf_url: String) -> Self {
        let page_url = swf_url.clone();
        Self { swf_url, page_url }
    }

    pub fn with_page_url(mut self, page_url: String) -> Self {
        self.page_url = page_url;
        self
    }
}

impl UrlProvider for AndroidUrlProvider {
    fn get_document_url(&self, _instance: i32) -> Option<String> {
        Some(self.page_url.clone())
    }

    fn get_plugin_instance_url(&self, _instance: i32) -> Option<String> {
        Some(self.swf_url.clone())
    }
}
