#[derive(Debug, serde::Serialize)]
struct ApplicationInfoResponse {
    name: String,
    version: String,
}

impl From<tule_core::ApplicationInfo> for ApplicationInfoResponse {
    fn from(info: tule_core::ApplicationInfo) -> Self {
        Self {
            name: info.name,
            version: info.version,
        }
    }
}

#[tauri::command]
fn get_application_info() -> ApplicationInfoResponse {
    tule_core::get_application_info().into()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![get_application_info])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn application_info_command_preserves_the_typed_core_shape() {
        let expected = tule_core::get_application_info();
        let response = get_application_info();
        let json = serde_json::to_value(response).expect("response should serialize");

        assert_eq!(
            json,
            serde_json::json!({
                "name": expected.name,
                "version": expected.version,
            })
        );
    }
}
