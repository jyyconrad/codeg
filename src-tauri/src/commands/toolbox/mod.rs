mod bcrypt;
mod bytes;
mod cert;
mod cipher;
mod hash;
mod jobs;

pub use bcrypt::{BCRYPT_COST_DEFAULT, BCRYPT_COST_MAX, BCRYPT_COST_MIN};
pub use cert::CertView;
pub use cipher::CipherFileParams;
pub use hash::HashReport;
pub use jobs::TOOLBOX_PROGRESS_EVENT;

use crate::app_error::AppCommandError;

#[cfg(feature = "tauri-runtime")]
mod tauri_commands {
    use std::path::PathBuf;

    use tauri::AppHandle;

    use super::*;
    use crate::web::event_bridge::EventEmitter;

    #[tauri::command]
    pub async fn toolbox_hash_file(
        path: String,
        job_id: String,
        app: AppHandle,
    ) -> Result<HashReport, AppCommandError> {
        let cancel = jobs::register(&job_id);
        let emitter = EventEmitter::Tauri(app);
        let job = job_id.clone();
        let result = tokio::task::spawn_blocking(move || {
            hash::hash_file_core(PathBuf::from(path).as_path(), &job, &emitter, &cancel)
        })
        .await
        .map_err(|e| AppCommandError::invalid_input(e.to_string()))?;
        jobs::finish(&job_id);
        result
    }

    #[tauri::command]
    pub async fn toolbox_cipher_file(
        params: CipherFileParams,
        app: AppHandle,
    ) -> Result<(), AppCommandError> {
        let job_id = params.job_id.clone();
        let cancel = jobs::register(&job_id);
        let emitter = EventEmitter::Tauri(app);
        let result = tokio::task::spawn_blocking(move || {
            cipher::cipher_file_core(&params, &emitter, &cancel)
        })
        .await
        .map_err(|e| AppCommandError::invalid_input(e.to_string()))?;
        jobs::finish(&job_id);
        result
    }

    #[tauri::command]
    pub fn toolbox_cancel_job(job_id: String) -> bool {
        jobs::cancel(&job_id)
    }

    #[tauri::command]
    pub async fn toolbox_bcrypt_hash(
        password: String,
        cost: Option<u32>,
        job_id: String,
    ) -> Result<String, AppCommandError> {
        let cancel = jobs::register(&job_id);
        let cost = cost.unwrap_or(BCRYPT_COST_DEFAULT);
        let result = tokio::task::spawn_blocking(move || {
            if jobs::is_cancelled(&cancel) {
                return Err(jobs::cancelled_error());
            }
            bcrypt::bcrypt_hash_core(&password, cost)
        })
        .await
        .map_err(|e| AppCommandError::invalid_input(e.to_string()))?;
        jobs::finish(&job_id);
        result
    }

    #[tauri::command]
    pub async fn toolbox_bcrypt_verify(
        password: String,
        hash: String,
    ) -> Result<bool, AppCommandError> {
        tokio::task::spawn_blocking(move || bcrypt::bcrypt_verify_core(&password, &hash))
            .await
            .map_err(|e| AppCommandError::invalid_input(e.to_string()))?
    }

    #[tauri::command]
    pub fn toolbox_parse_cert(
        pem: Option<String>,
        path: Option<String>,
    ) -> Result<CertView, AppCommandError> {
        if let Some(path) = path.filter(|p| !p.is_empty()) {
            return cert::parse_cert_file(PathBuf::from(path).as_path());
        }
        let pem = pem.unwrap_or_default();
        if pem.trim().is_empty() {
            return Err(AppCommandError::invalid_input(
                "Paste a PEM certificate or pick a file.",
            ));
        }
        cert::parse_cert_pem(&pem)
    }
}

#[cfg(feature = "tauri-runtime")]
pub use tauri_commands::*;
