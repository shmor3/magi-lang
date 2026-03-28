fn main() {
    println!("cargo::rustc-check-cfg=cfg(has_sdl2)");
    println!("cargo::rustc-check-cfg=cfg(has_pulseaudio)");

    // Embed build date (UTC, YYYY-MM-DD)
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    // Simple UTC date calculation
    let days = now / 86400;
    let (year, month, day) = days_to_date(days);
    println!("cargo:rustc-env=MAGI_BUILD_DATE={year:04}-{month:02}-{day:02}");

    // Embed target triple
    let target = std::env::var("TARGET").unwrap_or_else(|_| "unknown".to_string());
    println!("cargo:rustc-env=MAGI_BUILD_TARGET={target}");

    // Link OpenSSL for TLS support (HTTPS, WSS)
    println!("cargo:rustc-link-lib=ssl");
    println!("cargo:rustc-link-lib=crypto");

    // SDL2 for pixel graphics (optional — runtime dlopen if not linked)
    if probe_lib("SDL2") {
        println!("cargo:rustc-link-lib=SDL2");
        println!("cargo:rustc-cfg=has_sdl2");
    }

    // PulseAudio for real-time audio streaming (optional)
    if probe_lib("pulse-simple") {
        println!("cargo:rustc-link-lib=pulse-simple");
        println!("cargo:rustc-link-lib=pulse");
        println!("cargo:rustc-cfg=has_pulseaudio");
    }
}

/// Check if a C library is available by attempting to locate it.
fn probe_lib(name: &str) -> bool {
    // Try pkg-config first
    if let Ok(output) = std::process::Command::new("pkg-config")
        .args(["--exists", name])
        .output()
    {
        if output.status.success() {
            return true;
        }
    }
    // Fallback: check common library paths
    let lib_name = format!("lib{}.so", name);
    for dir in &["/usr/lib", "/usr/lib64", "/usr/local/lib", "/usr/lib/x86_64-linux-gnu"] {
        if std::path::Path::new(dir).join(&lib_name).exists() {
            return true;
        }
    }
    false
}

/// Convert days since Unix epoch to (year, month, day).
fn days_to_date(days: u64) -> (u64, u64, u64) {
    // Algorithm from https://howardhinnant.github.io/date_algorithms.html
    let z = days + 719468;
    let era = z / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}
