extern crate tauri as tauri_crate;

#[macro_export]
macro_rules! openmind_generate_handler {
    ($($command:ident),* $(,)?) => {
        $crate::tauri_crate::generate_handler![
            $($command),*,
            transcribe_audio,
            ocr_pdf_pages,
            send_multimodal_chat_message,
            regenerate_multimodal_message,
            artifact_media_data_url,
            create_soundscape_artifact
        ]
    };
}

mod tauri {
    pub use crate::openmind_generate_handler as generate_handler;
    pub use crate::tauri_crate::*;
}

mod multimodal;
mod pdf_ocr;
mod speech_runtime;
mod vision_batch;

pub(crate) use multimodal::{
    artifact_media_data_url, create_soundscape_artifact, regenerate_multimodal_message,
    send_multimodal_chat_message, transcribe_audio,
};
pub(crate) use pdf_ocr::ocr_pdf_pages;

include!("lib_legacy.rs");
