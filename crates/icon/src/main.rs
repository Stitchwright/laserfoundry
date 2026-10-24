//! Writes the application icon in a platform format: `laserfoundry-icon png|ico|icns <out> [size]`.

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (Some(format), Some(path)) = (args.first(), args.get(1)) else {
        eprintln!("usage: laserfoundry-icon png|ico|icns <out-file> [size]");
        return ExitCode::FAILURE;
    };
    let bytes = match format.as_str() {
        "png" => laserfoundry_icon::png(args.get(2).and_then(|s| s.parse().ok()).unwrap_or(256), 0.0),
        "ico" => laserfoundry_icon::ico(),
        "icns" => laserfoundry_icon::icns(),
        other => {
            eprintln!("unknown format {other}: use png, ico or icns");
            return ExitCode::FAILURE;
        }
    };
    if bytes.is_empty() {
        eprintln!("could not draw the icon");
        return ExitCode::FAILURE;
    }
    match std::fs::write(path, bytes) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{path}: {e}");
            ExitCode::FAILURE
        }
    }
}
