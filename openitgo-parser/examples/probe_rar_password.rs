//! Diagnostic probe: classify unrar 0.5 errors for encrypted RAR archives.
//! Usage:
//!   cargo run -p openitgo-parser --example probe_rar_password -- <rar-file> [password]
//! Run three times per fixture: no password / wrong password / correct password
//! and compare against the mapping table in the batch plan (Task 3 Step 2).

use std::path::Path;

fn list_and_read(path: &Path, password: Option<&str>) {
    let builder = match password {
        Some(pw) => unrar::Archive::with_password(path, pw),
        None => unrar::Archive::new(path),
    };
    match builder.open_for_listing() {
        Err(e) => println!("open_for_listing ERR code={:?} when={:?}", e.code, e.when),
        Ok(archive) => {
            for (i, entry) in archive.enumerate() {
                match entry {
                    Ok(h) => println!(
                        "header[{i}]: {} is_file={}",
                        h.filename.to_string_lossy(),
                        h.is_file()
                    ),
                    Err(e) => {
                        println!("header[{i}] ERR code={:?} when={:?}", e.code, e.when);
                        break;
                    }
                }
            }
        }
    }

    let builder = match password {
        Some(pw) => unrar::Archive::with_password(path, pw),
        None => unrar::Archive::new(path),
    };
    match builder.open_for_processing() {
        Err(e) => println!(
            "open_for_processing ERR code={:?} when={:?}",
            e.code, e.when
        ),
        Ok(mut archive) => loop {
            match archive.read_header() {
                Err(e) => {
                    println!("read_header ERR code={:?} when={:?}", e.code, e.when);
                    break;
                }
                Ok(None) => {
                    println!("read_header: end of archive");
                    break;
                }
                Ok(Some(entry)) => {
                    if entry.entry().is_file() {
                        match entry.read() {
                            Ok((bytes, _)) => println!("entry.read: OK {} bytes", bytes.len()),
                            Err(e) => {
                                println!("entry.read ERR code={:?} when={:?}", e.code, e.when)
                            }
                        }
                        break;
                    }
                    match entry.skip() {
                        Ok(a) => archive = a,
                        Err(e) => {
                            println!("entry.skip ERR code={:?} when={:?}", e.code, e.when);
                            break;
                        }
                    }
                }
            }
        },
    }
}

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: probe_rar_password <rar-file> [password]");
    let password = std::env::args().nth(2);
    println!("== {:?} ==", password);
    list_and_read(Path::new(&path), password.as_deref());
}
