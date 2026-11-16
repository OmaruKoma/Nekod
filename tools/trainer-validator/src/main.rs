use std::env;
use std::fs;
use std::process;

use nekod_validator::ct_importer::{CtImportOptions, convert_ct_file_to_package_dir};
use nekod_validator::{
    validate_declarative_patch_str, validate_game_manifest_str, validate_registry_manifest_str,
    validate_trainer_manifest_str, validate_trainer_package_dir,
};

fn print_usage() {
    eprintln!(
        "Usage:\n  nekod-validator <game|trainer|registry|patch|package> <path-to-manifest-or-dir>\n  nekod-validator import-ct <input.ct> --game-id <slug> --game-name <name> --trainer-id <id> --out <dir> [--name <title>] [--module <exe>] [--repo <https-url>]"
    );
}

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        print_usage();
        process::exit(2);
    }

    if args[1] == "import-ct" {
        run_import_ct(&args[2..]);
        return;
    }

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

fn run_import_ct(args: &[String]) {
    if args.is_empty() {
        print_usage();
        process::exit(2);
    }
    let input_ct = &args[0];
    let mut game_id: Option<String> = None;
    let mut game_name: Option<String> = None;
    let mut trainer_id: Option<String> = None;
    let mut trainer_name: Option<String> = None;
    let mut out_dir: Option<String> = None;
    let mut default_module: Option<String> = None;
    let mut source_repo = "https://github.com/nekod-org/Nekod".to_owned();

    let mut i = 1;
    while i < args.len() {
        let flag = args[i].as_str();
        let next = args.get(i + 1).cloned();
        match (flag, next) {
            ("--game-id", Some(v)) => game_id = Some(v),
            ("--game-name", Some(v)) => game_name = Some(v),
            ("--trainer-id", Some(v)) => trainer_id = Some(v),
            ("--name", Some(v)) => trainer_name = Some(v),
            ("--out", Some(v)) => out_dir = Some(v),
            ("--module", Some(v)) => default_module = Some(v),
            ("--repo", Some(v)) => source_repo = v,
            _ => {
                eprintln!("unknown or incomplete option '{flag}'");
                print_usage();
                process::exit(2);
            }
        }
        i += 2;
    }

    let (Some(game_id), Some(game_name), Some(trainer_id), Some(out_dir)) =
        (game_id, game_name, trainer_id, out_dir)
    else {
        eprintln!("import-ct requires --game-id, --game-name, --trainer-id, and --out");
        print_usage();
        process::exit(2);
    };

    let trainer_name = trainer_name.unwrap_or_else(|| format!("{game_name} Imported Trainer"));
    let opts = CtImportOptions {
        trainer_id,
        trainer_name,
        game_id,
        game_name,
        source_repository: source_repo,
        default_module,
    };

    match convert_ct_file_to_package_dir(input_ct, &out_dir, &opts) {
        Ok(res) => {
            println!(
                "imported {} feature(s) into '{}' (manifest sha256 {})",
                res.summary.converted_features.len(),
                out_dir,
                res.package.manifest_sha256
            );
            if !res.summary.skipped_entries.is_empty() {
                eprintln!(
                    "skipped {} unsafe/unsupported .CT entry(s):",
                    res.summary.skipped_entries.len()
                );
                for skip in &res.summary.skipped_entries {
                    eprintln!(
                        "  - [{}] {}: {}",
                        skip.variable_type, skip.description, skip.reason
                    );
                }
            }
        }
        Err(err) => {
            eprintln!("import-ct failed for '{input_ct}': {err}");
            process::exit(1);
        }
    }
}
