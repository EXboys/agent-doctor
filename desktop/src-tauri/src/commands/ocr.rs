use agent_doctor_ocr::{read_image_texts, ImageTextReport};

#[tauri::command]
pub fn read_image_texts_command(paths: Vec<String>) -> ImageTextReport {
    read_image_texts(paths)
}
