use std::env;
use std::fs;
use std::process;

use nekod_validator::{
    validate_declarative_patch_str, validate_game_manifest_str, validate_registry_manifest_str,
    validate_trainer_manifest_str, validate_trainer_package_dir,
};

fn print_usage() {
    eprintln!(
        "Usage: nekod-validator <game|trainer|registry|patch|package> <path-to-manifest-or-dir>"
    );
}

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() != 3 {
        print_usage();
        process::exit(2);
    }

    let kind = args[1].as_str();
    let path = &args[2];

    if kind == "package" {
        match validate_trainer_package_dir(path) {
            Ok((m, sha)) => {
                println!(
                    "valid trainer package: {}@{} (manifest sha256 {sha})",
                    m.id, m.version
                );
                return;
            }
            Err(err) => {
                eprintln!("package validation failed for '{path}':\n{err}");
                process::exit(1);
            }
        }
    }

    let raw = match fs::read_to_string(path) {
        Ok(content) => content,
        Err(err) => {
            eprintln!("error reading '{path}': {err}");
            process::exit(1);
        }
    };

    let result = match kind {
        "game" => {
            validate_game_manifest_str(&raw).map(|m| format!("valid game manifest: {}", m.id))
        }
        "trainer" => validate_trainer_manifest_str(&raw)
            .map(|m| format!("valid trainer manifest: {}@{}", m.id, m.version)),
        "registry" => validate_registry_manifest_str(&raw)
            .map(|m| format!("valid registry manifest: {}", m.repository_id)),
        "patch" => validate_declarative_patch_str(&raw).map(|p| {
            format!(
                "valid declarative patch payload ({} features)",
                p.features.len()
            )
        }),
        _ => {
            print_usage();
            process::exit(2);
        }
    };

    match result {
        Ok(msg) => println!("{msg}"),
        Err(err) => {
            eprintln!("validation failed for '{path}':\n{err}");
            process::exit(1);
        }
    }
}
