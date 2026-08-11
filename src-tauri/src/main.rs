// Tips is a desktop GUI application, so Windows builds should not open a console window.
#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

fn main() {
    tips_desktop_lib::run()
}
