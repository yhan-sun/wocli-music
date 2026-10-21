fn main() {
    let raw: Vec<String> = std::env::args().skip(1).collect();
    let json = raw.iter().any(|v| v == "--json");
    if let Err(error) = wocli_music::cli::run(raw) {
        if json {
            eprintln!(
                "{}",
                serde_json::json!({"ok":false,"error":error.message,"code":error.code,"details":error.details})
            );
        } else {
            eprintln!("错误：{error}");
        }
        std::process::exit(if error.code == "cancelled" { 130 } else { 1 });
    }
}
