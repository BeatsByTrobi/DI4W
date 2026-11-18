//! The island's last crash, kept in crash.txt until it's reported or dismissed.
//!
//! The island writes it when it goes down and offers to report it on the next start; nothing is ever sent without
//! the user opening the report and pressing the button themselves.

pub fn write(details: &str) {
    let text = format!(
        "{} (version {})\n{details}",
        chrono::Local::now().format("%Y-%m-%d %H:%M:%S"),
        crate::updates::current_version()
    );
    let _ = std::fs::create_dir_all(crate::paths::data_folder());
    let _ = std::fs::write(crate::paths::crash_file(), text); // the process is going down; nothing more to do
}

pub fn read() -> Option<String> {
    std::fs::read_to_string(crate::paths::crash_file()).ok()
}

pub fn clear() {
    let _ = std::fs::remove_file(crate::paths::crash_file());
}
