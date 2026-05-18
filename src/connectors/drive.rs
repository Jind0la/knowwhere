#[cfg(feature = "google-drive")]
use std::collections::HashMap;
#[cfg(feature = "google-drive")]
use std::time::Duration;

#[cfg(feature = "google-drive")]
use anyhow::{Context, Result};
#[cfg(feature = "google-drive")]
use google_drive3::hyper_rustls::{self, HttpsConnector};
#[cfg(feature = "google-drive")]
use google_drive3::hyper_util::client::legacy::connect::HttpConnector;
#[cfg(feature = "google-drive")]
use google_drive3::yup_oauth2::{self, ServiceAccountAuthenticator};
#[cfg(feature = "google-drive")]
use google_drive3::DriveHub;
#[cfg(feature = "google-drive")]
use serde_json::json;
#[cfg(feature = "google-drive")]
use tracing;

#[cfg(feature = "google-drive")]
use crate::multimodal::MultimodalData;

#[cfg(feature = "google-drive")]
use super::ExternalEvent;

/// Google Drive connector using the Changes API (polling strategy).
///
/// Uses a Google Cloud Service Account for authentication.
/// The service account must have read access to the target folder.
///
/// # Environment Variables
/// - `GOOGLE_SERVICE_ACCOUNT_PATH` — path to the service account JSON key file
/// - `GOOGLE_DRIVE_WATCH_FOLDER_ID` — optional folder ID to watch
/// - `GOOGLE_DRIVE_POLL_INTERVAL_SECS` — polling interval in seconds (default: 30)
#[cfg(feature = "google-drive")]
pub struct GoogleDriveConnector {
    hub: DriveHub<HttpsConnector<HttpConnector>>,
    watch_folder_id: Option<String>,
    poll_interval: Duration,
    /// The last page token from `changes().list()`.
    /// Persisted across polls; initialized on first poll via `get_start_page_token()`.
    page_token: Option<String>,
}

#[cfg(feature = "google-drive")]
impl GoogleDriveConnector {
    /// Create a new Google Drive connector.
    ///
    /// Reads `GOOGLE_SERVICE_ACCOUNT_PATH` from the environment.
    /// Falls back to `service-account.json` in the current directory if not set.
    pub async fn new(watch_folder_id: Option<String>) -> Result<Self> {
        let sa_path = std::env::var("GOOGLE_SERVICE_ACCOUNT_PATH")
            .unwrap_or_else(|_| "service-account.json".to_string());

        let secret = yup_oauth2::read_service_account_key(&sa_path)
            .await
            .with_context(|| format!("Failed to read service account key from {sa_path}"))?;

        let auth = ServiceAccountAuthenticator::builder(secret)
            .build()
            .await
            .context("Failed to build service account authenticator")?;

        let hub = DriveHub::new(
            hyper_util::client::legacy::Client::builder(hyper_util::rt::TokioExecutor::new())
                .build(
                    hyper_rustls::HttpsConnectorBuilder::new()
                        .with_native_roots()
                        .context("Failed to load native TLS roots")?
                        .https_or_http()
                        .enable_http1()
                        .build(),
                ),
            auth,
        );

        let poll_secs: u64 = std::env::var("GOOGLE_DRIVE_POLL_INTERVAL_SECS")
            .unwrap_or_else(|_| "30".to_string())
            .parse()
            .unwrap_or(30);

        Ok(Self {
            hub,
            watch_folder_id,
            poll_interval: Duration::from_secs(poll_secs),
            page_token: None,
        })
    }

    /// Poll the Google Drive Changes API for new or modified files.
    ///
    /// On first call, fetches the start page token. Subsequent calls use the
    /// saved token to fetch only new changes.
    ///
    /// Returns an empty `Vec` if no changes are found or on transient errors
    /// (following the same resilient pattern as FrigateConnector).
    pub async fn poll_changes(&mut self) -> Result<Vec<ExternalEvent>> {
        // Initialize page token on first poll
        if self.page_token.is_none() {
            match self.fetch_start_page_token().await {
                Ok(token) => {
                    tracing::info!("Google Drive: initialized start page token");
                    self.page_token = Some(token);
                }
                Err(e) => {
                    tracing::warn!(error = %e, "Google Drive: failed to get start page token");
                    return Ok(vec![]);
                }
            }
        }

        let token = self.page_token.as_ref().expect("page_token just set");

        // Build the changes.list request
        let request = self
            .hub
            .changes()
            .list(token)
            .include_corpus_removals(false)
            .page_size(100)
            .supports_all_drives(true)
            .param(
                "fields",
                "newStartPageToken,nextPageToken,changes(file(id,name,mimeType,parents,size,createdTime,modifiedTime),type,changeType)",
            );

        let (_, changes_response) = match request.doit().await {
            Ok(resp) => resp,
            Err(e) => {
                tracing::warn!(error = %e, "Google Drive: changes.list failed, resetting token");
                self.page_token = None; // Reset token so we re-initialize next time
                return Ok(vec![]);
            }
        };

        // Update page token for next poll
        if let Some(new_token) = changes_response.new_start_page_token {
            self.page_token = Some(new_token);
        } else if let Some(next_token) = changes_response.next_page_token {
            self.page_token = Some(next_token);
        }

        let Some(changes) = changes_response.changes else {
            return Ok(vec![]);
        };

        let mut events = Vec::new();
        for change in changes {
            let Some(file) = change.file else {
                continue;
            };

            // Skip folders and trashed files
            if file.mime_type.as_deref() == Some("application/vnd.google-apps.folder") {
                continue;
            }
            if file.trashed == Some(true) {
                continue;
            }

            // If watching a specific folder, filter by parent
            if let Some(ref folder_id) = self.watch_folder_id {
                let is_in_folder = file.parents.as_ref().map_or(false, |parents| {
                    parents.contains(folder_id)
                });
                if !is_in_folder {
                    continue;
                }
            }

            let file_id = file.id.as_deref().unwrap_or("unknown");
            let pointer = format!("gdrive://file/{file_id}");

            let name = file.name.as_deref().unwrap_or("unknown");
            let mime_type = file.mime_type.as_deref().unwrap_or("application/octet-stream");
            let change_type = change
                .change_type
                .as_deref()
                .unwrap_or("unknown");

            let mut metadata = HashMap::from([
                ("source".to_string(), json!("google_drive")),
                ("name".to_string(), json!(name)),
                ("mime_type".to_string(), json!(mime_type)),
                ("file_id".to_string(), json!(file_id)),
                ("change_type".to_string(), json!(change_type)),
            ]);

            if let Some(s) = file.size {
                metadata.insert("size_bytes".to_string(), json!(s));
            }

            let multimodal = if mime_type.starts_with("image/")
                || mime_type.starts_with("audio/")
                || mime_type.starts_with("video/")
            {
                Some(MultimodalData::Image {
                    pointer: pointer.clone(),
                    embedding: vec![], // Cross-modal embedding done by the embedding provider
                })
            } else {
                None
            };

            events.push(ExternalEvent {
                pointer,
                metadata,
                multimodal,
                created_at: None,
            });
        }

        tracing::debug!(count = events.len(), "Google Drive: polled changes");
        Ok(events)
    }

    /// Fetch the start page token from the Changes API.
    async fn fetch_start_page_token(&self) -> Result<String> {
        let (_, token_response) = self
            .hub
            .changes()
            .get_start_page_token()
            .supports_all_drives(true)
            .doit()
            .await
            .context("Failed to get start page token")?;

        token_response
            .start_page_token
            .context("No start page token in response")
    }
}
