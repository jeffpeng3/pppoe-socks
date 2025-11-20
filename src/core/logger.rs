use chrono::Local;
use env_logger::Builder;
use env_logger::fmt::style::{AnsiColor, Style};
use std::io::Write;

pub fn init() {
    Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .format(|buf, record| {
            let subtle = Style::new().fg_color(Some(AnsiColor::BrightBlack.into()));
            let level_style = buf.default_level_style(record.level());

            writeln!(
                buf,
                "{subtle}[{subtle:#}{} {level_style}{:<5}{level_style:#} {}{subtle}]{subtle:#} {}",
                Local::now().format("%m-%d %H:%M:%S%:::z"),
                record.level(),
                record.module_path().unwrap_or("<unknown>"),
                record.args(),
            )
        })
        .filter_module("serenity", log::LevelFilter::Warn)
        .filter_module("tracing", log::LevelFilter::Warn)
        .init();
}
