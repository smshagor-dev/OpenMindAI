extern crate tauri as tauri_crate;

#[macro_export]
macro_rules! openmind_generate_handler {
    ($($command:ident),* $(,)?) => {
        $crate::tauri_crate::generate_handler![
            $($command),*,
            transcribe_audio,
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
mod speech_runtime;

pub(crate) use multimodal::{
    artifact_media_data_url, create_soundscape_artifact, regenerate_multimodal_message,
    send_multimodal_chat_message, transcribe_audio,
};

include!("lib_legacy.rs");
