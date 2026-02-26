use std::collections::HashMap;

use anyhow::Result;
use serde_json::json;

use super::ExternalEvent;

pub struct GoogleDriveConnector {
    pub watch_folder_id: Option<String>,
}

impl GoogleDriveConnector {
    pub fn new(watch_folder_id: Option<String>) -> Self {
        Self { watch_folder_id }
    }

    /// Placeholder: returns dummy drive change events.
    /// Will be replaced with Google Drive Changes API + push notifications.
    pub async fn poll_changes(&self) -> Result<Vec<ExternalEvent>> {
        let file_id = uuid::Uuid::new_v4();
        let pointer = format!("gdrive://file/{file_id}");

        let event = ExternalEvent {
            pointer,
            metadata: HashMap::from([
                ("source".to_string(), json!("google_drive")),
                ("mime_type".to_string(), json!("application/pdf")),
                ("name".to_string(), json!("project_notes.pdf")),
            ]),
            multimodal: None,
        };

        Ok(vec![event])
    }
}
