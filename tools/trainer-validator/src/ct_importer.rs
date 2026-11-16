//! Static Cheat Engine `.CT` (XML) -> Nekod `declarative_patch_v1` converter.
//!
//! Converts safe declarative `.CT` entries (module-relative scalars, bounded pointer
//! chains, and pure `[ENABLE]`/`[DISABLE]` `aobscanmodule` + `db` byte patches) into
//! validated `TrainerManifest` + `DeclarativePatchPayload` packages while failing closed
//! on arbitrary Lua (`{$lua}`), dynamic memory allocation (`alloc`), threads, or raw
//! assembly injections.

use std::collections::BTreeSet;
use std::path::Path;

use nekod_manifest::{FeatureCategory, FeatureControlType, FeatureValueRange, is_valid_kebab_slug};
use nekod_trainer_sdk::{
    AddressLocator, DeclarativePatchBuilder, GeneratedTrainerPackage, ScalarValueType,
    TrainerManifestBuilder, build_trainer_package,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{ValidationError, validate_trainer_package_dir};

/// Options controlling `.CT` to `declarative_patch_v1` package generation.
#[derive(Debug, Clone)]
pub struct CtImportOptions {
    pub trainer_id: String,
    pub trainer_name: String,
    pub game_id: String,
    pub game_name: String,
    pub source_repository: String,
    pub default_module: Option<String>,
}

/// Reason an individual `.CT` entry could not be safely converted statically.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkippedCtEntry {
    pub description: String,
    pub variable_type: String,
    pub reason: String,
}

/// Summary returned after static `.CT` conversion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CtImportSummary {
    pub converted_features: Vec<String>,
    pub skipped_entries: Vec<SkippedCtEntry>,
    pub modules: Vec<String>,
}

/// Full output produced by [`convert_ct_xml_to_package`].
#[derive(Debug, Clone)]
pub struct CtConversionOutput {
    pub package: GeneratedTrainerPackage,
    pub summary: CtImportSummary,
}

#[derive(Debug, Error)]
pub enum CtImportError {
    #[error("invalid `.CT` XML: {0}")]
    XmlParse(String),

    #[error("invalid package metadata: {0}")]
    Metadata(String),

    #[error("no convertible declarative features found in `.CT` file ({0} skipped)")]
    NoConvertibleFeatures(usize),

    #[error("validation failed after conversion: {0}")]
    Validation(#[from] ValidationError),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Debug, Clone, Default)]
struct RawCtEntry {
    description: String,
    variable_type: String,
    address: Option<String>,
    offsets: Vec<String>,
    assembler_script: Option<String>,
    children: Vec<RawCtEntry>,
}

/// Converts a raw Cheat Engine `.CT` XML document into a validated `GeneratedTrainerPackage`
/// along with explicit diagnostics for any skipped/unsafe entries.
pub fn convert_ct_xml_to_package(
    ct_xml: &str,
    opts: &CtImportOptions,
) -> Result<CtConversionOutput, CtImportError> {
    if !is_valid_kebab_slug(&opts.game_id) {
        return Err(CtImportError::Metadata(format!(
            "game_id '{}' must be a valid U11 kebab-case slug",
            opts.game_id
        )));
    }

    let entries = parse_ct_entries(ct_xml)?;
    let mut flat_entries = Vec::new();
    flatten_entries(&entries, &mut flat_entries);

    let mut manifest_builder = TrainerManifestBuilder::new(
        opts.trainer_id.clone(),
        opts.trainer_name.clone(),
        opts.game_id.clone(),
        opts.game_name.clone(),
        opts.source_repository.clone(),
    )
    .description(format!(
        "Declarative trainer statically imported from Cheat Engine table for {}.",
        opts.game_name
    ))
    .author("CT Importer Contributor", None, None);

    let mut patch_builder = DeclarativePatchBuilder::new();
    let mut used_ids = BTreeSet::new();
    let mut modules = BTreeSet::new();
    let mut converted_features = Vec::new();
    let mut skipped_entries = Vec::new();
    let mut max_write_bytes: u64 = 8;

    for entry in flat_entries {
        let desc = clean_ct_label(&entry.description);
        let var_type = entry.variable_type.trim();
        let var_lower = var_type.to_ascii_lowercase();

        if var_lower.is_empty()
            || var_lower == "group"
            || entry.address.is_none() && entry.assembler_script.is_none()
        {
            continue;
        }

        if var_lower == "auto assembler script" {
            let script = entry.assembler_script.as_deref().unwrap_or("");
            match convert_auto_assembler_script(script, opts.default_module.as_deref()) {
                Ok(parsed) => {
                    let fid = unique_feature_id(&desc, &mut used_ids);
                    modules.insert(parsed.module.clone());
                    max_write_bytes = max_write_bytes.max(parsed.patch_len as u64);
                    patch_builder = patch_builder.byte_patch(
                        fid.clone(),
                        AddressLocator::AobPattern {
                            module: parsed.module,
                            pattern: parsed.aob_pattern,
                            match_index: 0,
                            offset: parsed.offset,
                        },
                        Some(parsed.original_bytes),
                        parsed.patch_bytes,
                    );
                    manifest_builder = manifest_builder.toggle_feature(
                        fid.clone(),
                        if desc.is_empty() { fid.clone() } else { desc },
                        "Imported declarative AOB byte patch from .CT entry.",
                        FeatureCategory::Other,
                        None,
                    );
                    converted_features.push(fid);
                }
                Err(reason) => skipped_entries.push(SkippedCtEntry {
                    description: desc,
                    variable_type: var_type.to_owned(),
                    reason,
                }),
            }
            continue;
        }

        let Some((scalar_type, control_type, byte_len)) = map_ct_variable_type(&var_lower) else {
            skipped_entries.push(SkippedCtEntry {
                description: desc,
                variable_type: var_type.to_owned(),
                reason: format!("unsupported Cheat Engine variable type '{var_type}'"),
            });
            continue;
        };

        let raw_addr = entry.address.as_deref().unwrap_or("").trim();
        match parse_module_offset(raw_addr, opts.default_module.as_deref()) {
            Ok((module, base_offset)) => {
                let locator = if entry.offsets.is_empty() {
                    AddressLocator::ModuleOffset {
                        module: module.clone(),
                        offset: base_offset,
                    }
                } else {
                    match parse_ct_pointer_offsets(&entry.offsets) {
                        Ok(offsets) => AddressLocator::PointerChain {
                            module: module.clone(),
                            base_offset,
                            offsets,
                        },
                        Err(reason) => {
                            skipped_entries.push(SkippedCtEntry {
                                description: desc,
                                variable_type: var_type.to_owned(),
                                reason,
                            });
                            continue;
                        }
                    }
                };

                let fid = unique_feature_id(&desc, &mut used_ids);
                modules.insert(module);
                max_write_bytes = max_write_bytes.max(byte_len);
                patch_builder =
                    patch_builder.write_value(fid.clone(), locator, scalar_type, None, Some(100));

                let label = if desc.is_empty() { fid.clone() } else { desc };
                manifest_builder = match control_type {
                    FeatureControlType::Integer => manifest_builder.integer_feature(
                        fid.clone(),
                        label,
                        "Imported scalar address entry from .CT table.",
                        FeatureCategory::Other,
                        None,
                        FeatureValueRange {
                            min: 0.0,
                            max: 999_999_999.0,
                            step: 1.0,
                            default: 100.0,
                        },
                    ),
                    _ => manifest_builder.feature(nekod_manifest::TrainerFeature {
                        id: fid.clone(),
                        name: label,
                        description: "Imported float address entry from .CT table.".to_owned(),
                        category: FeatureCategory::Other,
                        control_type: FeatureControlType::Float,
                        default_hotkey: None,
                        value_range: Some(FeatureValueRange {
                            min: 0.0,
                            max: 999_999.0,
                            step: 0.5,
                            default: 100.0,
                        }),
                        mutates_memory: true,
                        supports_rollback: true,
                        risk_level: nekod_manifest::FeatureRiskLevel::Low,
                    }),
                };
                converted_features.push(fid);
            }
            Err(reason) => skipped_entries.push(SkippedCtEntry {
                description: desc,
                variable_type: var_type.to_owned(),
                reason,
            }),
        }
    }

    if converted_features.is_empty() {
        return Err(CtImportError::NoConvertibleFeatures(skipped_entries.len()));
    }

    let module_list: Vec<String> = modules.into_iter().collect();
    let manifest = manifest_builder
        .default_memory_permissions(module_list.clone(), max_write_bytes.min(64))
        .build_draft();
    let payload = patch_builder.build().map_err(ValidationError::from)?;
    let package = build_trainer_package(manifest, payload).map_err(ValidationError::from)?;

    Ok(CtConversionOutput {
        package,
        summary: CtImportSummary {
            converted_features,
            skipped_entries,
            modules: module_list,
        },
    })
}

/// Converts a `.CT` file on disk, writes the validated package directory, and runs
/// `validate_trainer_package_dir` to guarantee end-to-end schema & SHA-256 compliance.
pub fn convert_ct_file_to_package_dir(
    ct_path: impl AsRef<Path>,
    out_dir: impl AsRef<Path>,
    opts: &CtImportOptions,
) -> Result<CtConversionOutput, CtImportError> {
    let raw_xml = std::fs::read_to_string(ct_path)?;
    let out = convert_ct_xml_to_package(&raw_xml, opts)?;
    out.package.write_to_dir(&out_dir)?;
    validate_trainer_package_dir(&out_dir)?;
    Ok(out)
}

fn flatten_entries<'a>(entries: &'a [RawCtEntry], out: &mut Vec<&'a RawCtEntry>) {
    for entry in entries {
        out.push(entry);
        flatten_entries(&entry.children, out);
    }
}

fn clean_ct_label(raw: &str) -> String {
    let t = raw.trim();
    if t.len() >= 2 && t.starts_with('"') && t.ends_with('"') {
        t[1..t.len() - 1].trim().to_owned()
    } else {
        t.to_owned()
    }
}

fn unique_feature_id(label: &str, used: &mut BTreeSet<String>) -> String {
    let mut slug = String::new();
    let mut prev_hyphen = false;
    for ch in label.chars() {
        let low = ch.to_ascii_lowercase();
        if low.is_ascii_alphanumeric() {
            slug.push(low);
            prev_hyphen = false;
        } else if !prev_hyphen && !slug.is_empty() {
            slug.push('-');
            prev_hyphen = true;
        }
    }
    while slug.ends_with('-') {
        slug.pop();
    }
    if slug.len() < 2 {
        slug = "ct-feature".to_owned();
    }
    if slug.len() > 48 {
        slug.truncate(48);
        while slug.ends_with('-') {
            slug.pop();
        }
    }

    let base = slug;
    let mut candidate = base.clone();
    let mut counter = 2usize;
    while !used.insert(candidate.clone()) {
        candidate = format!("{base}-{counter}");
        counter += 1;
    }
    candidate
}

fn map_ct_variable_type(lower: &str) -> Option<(ScalarValueType, FeatureControlType, u64)> {
    match lower {
        "4 bytes" => Some((ScalarValueType::I32, FeatureControlType::Integer, 4)),
        "8 bytes" => Some((ScalarValueType::I64, FeatureControlType::Integer, 8)),
        "float" => Some((ScalarValueType::F32, FeatureControlType::Float, 4)),
        "double" => Some((ScalarValueType::F64, FeatureControlType::Float, 8)),
        _ => None,
    }
}

fn parse_module_offset(raw: &str, default_module: Option<&str>) -> Result<(String, u64), String> {
    if raw.is_empty() {
        return Err("missing address".to_owned());
    }
    if let Some((left, right)) = raw.split_once('+') {
        let module = clean_ct_label(left);
        if module.is_empty() {
            return Err(format!("missing module name in address '{raw}'"));
        }
        let hex = right
            .trim()
            .trim_start_matches("0x")
            .trim_start_matches("0X");
        let offset = u64::from_str_radix(hex, 16)
            .map_err(|_| format!("invalid hex offset '{right}' in address '{raw}'"))?;
        Ok((module, offset))
    } else if raw.ends_with(".exe") || raw.ends_with(".dll") {
        Ok((clean_ct_label(raw), 0))
    } else if let Some(module) = default_module {
        let hex = raw.trim().trim_start_matches("0x").trim_start_matches("0X");
        if let Ok(offset) = u64::from_str_radix(hex, 16) {
            return Err(format!(
                "unanchored absolute hex address '0x{offset:X}' cannot safely target '{module}' without a module+offset or AOB"
            ));
        }
        Err(format!("unsupported address expression '{raw}'"))
    } else {
        Err(format!(
            "unanchored address '{raw}' (expected '\"Module.exe\"+HEX')"
        ))
    }
}

/// CE `<Offsets>` stores offsets bottom-up relative to pointer resolution; reverse into
/// outer-to-inner dereference order for `AddressLocator::PointerChain`.
fn parse_ct_pointer_offsets(raw_offsets: &[String]) -> Result<Vec<i64>, String> {
    if raw_offsets.len() > nekod_trainer_sdk::MAX_POINTER_CHAIN_DEPTH {
        return Err(format!(
            "pointer chain depth {} exceeds limit {}",
            raw_offsets.len(),
            nekod_trainer_sdk::MAX_POINTER_CHAIN_DEPTH
        ));
    }
    let mut out = Vec::with_capacity(raw_offsets.len());
    for raw in raw_offsets.iter().rev() {
        let trimmed = raw.trim();
        let (neg, digits) = if let Some(rest) = trimmed.strip_prefix('-') {
            (true, rest)
        } else {
            (false, trimmed)
        };
        let digits = digits.trim_start_matches("0x").trim_start_matches("0X");
        let mag = i64::from_str_radix(digits, 16)
            .map_err(|_| format!("invalid pointer offset '{raw}'"))?;
        out.push(if neg { -mag } else { mag });
    }
    Ok(out)
}

struct ParsedAaPatch {
    module: String,
    aob_pattern: String,
    offset: i64,
    patch_bytes: String,
    original_bytes: String,
    patch_len: usize,
}

fn convert_auto_assembler_script(
    script: &str,
    default_module: Option<&str>,
) -> Result<ParsedAaPatch, String> {
    let lower = script.to_ascii_lowercase();
    for forbidden in [
        "{$lua}",
        "alloc(",
        "createthread(",
        "loadlibrary",
        "fullaccess(",
    ] {
        if lower.contains(forbidden) {
            return Err(format!(
                "script contains '{forbidden}', which requires runtime execution rather than static declarative_patch_v1"
            ));
        }
    }

    let (enable_sec, disable_sec) = split_enable_disable(script)?;
    let (symbol, module, aob_pattern) = parse_aobscan(enable_sec, default_module)?;
    let (en_offset, patch_bytes, patch_len) = extract_db_patch(enable_sec, &symbol)?;
    let (dis_offset, original_bytes, orig_len) = extract_db_patch(disable_sec, &symbol)?;

    if en_offset != dis_offset {
        return Err(format!(
            "[ENABLE] patch offset ({en_offset}) differs from [DISABLE] offset ({dis_offset})"
        ));
    }
    if patch_len != orig_len {
        return Err(format!(
            "[ENABLE] byte count ({patch_len}) differs from [DISABLE] byte count ({orig_len})"
        ));
    }

    Ok(ParsedAaPatch {
        module,
        aob_pattern,
        offset: en_offset,
        patch_bytes,
        original_bytes,
        patch_len,
    })
}

fn split_enable_disable(script: &str) -> Result<(&str, &str), String> {
    let upper = script.to_ascii_uppercase();
    let en_idx = upper
        .find("[ENABLE]")
        .ok_or_else(|| "missing [ENABLE] section".to_owned())?;
    let dis_idx = upper
        .find("[DISABLE]")
        .ok_or_else(|| "missing [DISABLE] section".to_owned())?;
    if dis_idx <= en_idx {
        return Err("[DISABLE] must follow [ENABLE]".to_owned());
    }
    Ok((
        &script[en_idx + "[ENABLE]".len()..dis_idx],
        &script[dis_idx + "[DISABLE]".len()..],
    ))
}

fn parse_aobscan(
    enable_sec: &str,
    default_module: Option<&str>,
) -> Result<(String, String, String), String> {
    for raw_line in enable_sec.lines() {
        let line = strip_aa_comment(raw_line).trim();
        let lower = line.to_ascii_lowercase();
        if let Some(rest) = lower
            .strip_prefix("aobscanmodule(")
            .and_then(|_| line.get("aobscanmodule(".len()..))
            && let Some(inner) = rest.strip_suffix(')')
        {
            let parts: Vec<&str> = inner.split(',').map(str::trim).collect();
            if parts.len() == 3 {
                return Ok((
                    parts[0].to_owned(),
                    clean_ct_label(parts[1]),
                    normalize_aob(parts[2])?,
                ));
            }
        }
        if lower.starts_with("aobscan(")
            && let Some(inner) = line
                .get("aobscan(".len()..)
                .and_then(|s| s.strip_suffix(')'))
        {
            let parts: Vec<&str> = inner.split(',').map(str::trim).collect();
            if parts.len() == 2 {
                let module = default_module.ok_or_else(|| {
                    "aobscan() without module requires --module fallback".to_owned()
                })?;
                return Ok((
                    parts[0].to_owned(),
                    module.to_owned(),
                    normalize_aob(parts[1])?,
                ));
            }
        }
    }
    Err("no supported aobscanmodule(SYM, Module, Pattern) found in [ENABLE]".to_owned())
}

fn normalize_aob(raw: &str) -> Result<String, String> {
    let mut tokens = Vec::new();
    for tok in raw.split_whitespace() {
        if tok == "?" || tok == "??" || tok == "*" {
            tokens.push("??".to_owned());
        } else if tok.len() == 2 && tok.bytes().all(|b| b.is_ascii_hexdigit()) {
            tokens.push(tok.to_ascii_uppercase());
        } else {
            return Err(format!("invalid AOB token '{tok}'"));
        }
    }
    if tokens.is_empty() {
        return Err("empty AOB pattern".to_owned());
    }
    Ok(tokens.join(" "))
}

fn extract_db_patch(section: &str, symbol: &str) -> Result<(i64, String, usize), String> {
    let mut matched_offset: Option<i64> = None;
    let mut db_bytes = Vec::new();

    for raw_line in section.lines() {
        let line = strip_aa_comment(raw_line).trim();
        if line.is_empty() {
            continue;
        }
        let lower = line.to_ascii_lowercase();
        if lower.starts_with("aobscanmodule(")
            || lower.starts_with("aobscan(")
            || lower.starts_with("registersymbol(")
            || lower.starts_with("unregistersymbol(")
            || lower.starts_with("define(")
        {
            continue;
        }

        if let Some(label_expr) = line.strip_suffix(':') {
            let label = label_expr.trim();
            if label == symbol {
                matched_offset = Some(0);
            } else if let Some(rest) = label.strip_prefix(symbol) {
                let rest = rest.trim();
                let (neg, hex) = if let Some(h) = rest.strip_prefix('+') {
                    (false, h.trim())
                } else if let Some(h) = rest.strip_prefix('-') {
                    (true, h.trim())
                } else {
                    return Err(format!("unsupported label expression '{label}'"));
                };
                let hex = hex.trim_start_matches("0x").trim_start_matches("0X");
                let val = i64::from_str_radix(hex, 16)
                    .map_err(|_| format!("invalid label offset '{label}'"))?;
                matched_offset = Some(if neg { -val } else { val });
            } else {
                return Err(format!("unsupported target label '{label}'"));
            }
            continue;
        }

        if matched_offset.is_some()
            && let Some(rest) = lower.strip_prefix("db").and_then(|_| line.get(2..))
            && (rest.is_empty() || rest.starts_with(char::is_whitespace))
        {
            for tok in rest.split(|c: char| c.is_whitespace() || c == ',') {
                let tok = tok.trim();
                if tok.is_empty() {
                    continue;
                }
                if tok.len() == 2 && tok.bytes().all(|b| b.is_ascii_hexdigit()) {
                    db_bytes.push(tok.to_ascii_uppercase());
                } else {
                    return Err(format!("unsupported non-byte db operand '{tok}'"));
                }
            }
            continue;
        }

        if matched_offset.is_some() {
            return Err(format!(
                "unsupported Auto-Assembler instruction '{line}'; only declarative `db` byte patches are statically convertible"
            ));
        }
    }

    let offset = matched_offset.ok_or_else(|| format!("missing patch label '{symbol}:'"))?;
    if db_bytes.is_empty() {
        return Err("missing `db` byte sequence".to_owned());
    }
    let count = db_bytes.len();
    Ok((offset, db_bytes.join(" "), count))
}

fn strip_aa_comment(line: &str) -> &str {
    line.split("//").next().unwrap_or(line)
}

// --- Minimal strict Cheat Engine XML reader (supports CDATA & standard XML entities) ---

fn parse_ct_entries(xml: &str) -> Result<Vec<RawCtEntry>, CtImportError> {
    if let Some(rel) = xml.find("<CheatEntries>") {
        let body_start = rel + "<CheatEntries>".len();
        let (entries, _) = parse_cheat_entries_body(xml, body_start)?;
        return Ok(entries);
    }
    Err(CtImportError::XmlParse(
        "missing <CheatEntries> root element".to_owned(),
    ))
}

fn parse_cheat_entries_body(
    xml: &str,
    mut pos: usize,
) -> Result<(Vec<RawCtEntry>, usize), CtImportError> {
    let mut entries = Vec::new();
    while pos < xml.len() {
        skip_ws_and_comments(xml, &mut pos);
        if xml[pos..].starts_with("</CheatEntries>") {
            return Ok((entries, pos + "</CheatEntries>".len()));
        }
        if xml[pos..].starts_with("<CheatEntry>") {
            pos += "<CheatEntry>".len();
            let (entry, next_pos) = parse_single_cheat_entry(xml, pos)?;
            entries.push(entry);
            pos = next_pos;
        } else if let Some(next_tag) = xml[pos..].find('<') {
            pos += next_tag.max(1);
        } else {
            break;
        }
    }
    Err(CtImportError::XmlParse(
        "unclosed <CheatEntries>".to_owned(),
    ))
}

fn parse_single_cheat_entry(
    xml: &str,
    mut pos: usize,
) -> Result<(RawCtEntry, usize), CtImportError> {
    let mut entry = RawCtEntry::default();
    while pos < xml.len() {
        skip_ws_and_comments(xml, &mut pos);
        if xml[pos..].starts_with("</CheatEntry>") {
            return Ok((entry, pos + "</CheatEntry>".len()));
        }
        if !xml[pos..].starts_with('<') {
            pos += 1;
            continue;
        }
        let close_angle = xml[pos..]
            .find('>')
            .ok_or_else(|| CtImportError::XmlParse("malformed XML tag".to_owned()))?;
        let tag_header = &xml[pos + 1..pos + close_angle];
        pos += close_angle + 1;

        if tag_header.ends_with('/') {
            continue;
        }
        let tag_name = tag_header.split_whitespace().next().unwrap_or("");
        if tag_name == "CheatEntries" {
            let (children, next_pos) = parse_cheat_entries_body(xml, pos)?;
            entry.children.extend(children);
            pos = next_pos;
            continue;
        }
        if tag_name == "Offsets" {
            let (offsets, next_pos) = parse_offsets_body(xml, pos)?;
            entry.offsets = offsets;
            pos = next_pos;
            continue;
        }

        let closing = format!("</{tag_name}>");
        let end_rel = xml[pos..]
            .find(&closing)
            .ok_or_else(|| CtImportError::XmlParse(format!("missing closing {closing}")))?;
        let raw_text = &xml[pos..pos + end_rel];
        pos += end_rel + closing.len();

        let text = decode_xml_text(raw_text);
        match tag_name {
            "Description" => entry.description = text,
            "VariableType" => entry.variable_type = text,
            "Address" => entry.address = Some(text),
            "AssemblerScript" => entry.assembler_script = Some(text),
            _ => {}
        }
    }
    Err(CtImportError::XmlParse("unclosed <CheatEntry>".to_owned()))
}

fn parse_offsets_body(xml: &str, mut pos: usize) -> Result<(Vec<String>, usize), CtImportError> {
    let mut offsets = Vec::new();
    while pos < xml.len() {
        skip_ws_and_comments(xml, &mut pos);
        if xml[pos..].starts_with("</Offsets>") {
            return Ok((offsets, pos + "</Offsets>".len()));
        }
        if xml[pos..].starts_with("<Offset>") {
            pos += "<Offset>".len();
            let end_rel = xml[pos..]
                .find("</Offset>")
                .ok_or_else(|| CtImportError::XmlParse("unclosed <Offset>".to_owned()))?;
            offsets.push(decode_xml_text(&xml[pos..pos + end_rel]).trim().to_owned());
            pos += end_rel + "</Offset>".len();
        } else {
            pos += 1;
        }
    }
    Err(CtImportError::XmlParse("unclosed <Offsets>".to_owned()))
}

fn skip_ws_and_comments(xml: &str, pos: &mut usize) {
    loop {
        while *pos < xml.len() && xml.as_bytes()[*pos].is_ascii_whitespace() {
            *pos += 1;
        }
        if xml[*pos..].starts_with("<!--")
            && let Some(end) = xml[*pos..].find("-->")
        {
            *pos += end + 3;
            continue;
        }
        if xml[*pos..].starts_with("<?")
            && let Some(end) = xml[*pos..].find("?>")
        {
            *pos += end + 2;
            continue;
        }
        break;
    }
}

fn decode_xml_text(raw: &str) -> String {
    let trimmed = raw.trim();
    if let Some(inner) = trimmed
        .strip_prefix("<![CDATA[")
        .and_then(|s| s.strip_suffix("]]>"))
    {
        return inner.to_owned();
    }
    raw.replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn converts_safe_ct_entries_and_skips_lua_or_alloc_safely() {
        let ct_xml = r#"<?xml version="1.0" encoding="utf-8"?>
<CheatTable CheatEngineTableVersion="45">
  <CheatEntries>
    <CheatEntry>
      <ID>0</ID>
      <Description>"Player Gold"</Description>
      <VariableType>4 Bytes</VariableType>
      <Address>"SampleRPG.exe"+0012ABCD</Address>
      <Offsets>
        <Offset>10</Offset>
        <Offset>28</Offset>
      </Offsets>
    </CheatEntry>
    <CheatEntry>
      <ID>1</ID>
      <Description>"Infinite Health"</Description>
      <VariableType>Auto Assembler Script</VariableType>
      <AssemblerScript>[ENABLE]
aobscanmodule(HEALTH_SUB,SampleRPG.exe,29 73 10 8B 43 10)
HEALTH_SUB:
  db 90 90 90
[DISABLE]
HEALTH_SUB:
  db 29 73 10
</AssemblerScript>
    </CheatEntry>
    <CheatEntry>
      <ID>2</ID>
      <Description>"Lua Code Cave (Unsafe)"</Description>
      <VariableType>Auto Assembler Script</VariableType>
      <AssemblerScript>{$lua}
print("hello")
[ENABLE]
alloc(newmem,1024)
[DISABLE]
dealloc(newmem)
</AssemblerScript>
    </CheatEntry>
  </CheatEntries>
</CheatTable>"#;

        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let out_dir =
            std::env::temp_dir().join(format!("nekod_ct_import_{}_{}", std::process::id(), nanos));

        let opts = CtImportOptions {
            trainer_id: "dev.example.ct-import".to_owned(),
            trainer_name: "Sample CT Import".to_owned(),
            game_id: "sample-singleplayer-rpg".to_owned(),
            game_name: "Sample Singleplayer RPG".to_owned(),
            source_repository: "https://github.com/example/ct-import".to_owned(),
            default_module: Some("SampleRPG.exe".to_owned()),
        };

        let result = convert_ct_xml_to_package(ct_xml, &opts).unwrap();
        assert_eq!(result.summary.converted_features.len(), 2);
        assert_eq!(result.summary.skipped_entries.len(), 1);
        assert!(result.summary.skipped_entries[0].reason.contains("{$lua}"));

        // Verify CE bottom-up offsets <Offset>10</Offset><Offset>28</Offset> reversed to [0x28, 0x10].
        let gold_feat = result.package.payload.feature("player-gold").unwrap();
        match gold_feat.operations[0].locator() {
            AddressLocator::PointerChain {
                module,
                base_offset,
                offsets,
            } => {
                assert_eq!(module, "SampleRPG.exe");
                assert_eq!(*base_offset, 0x0012_ABCD);
                assert_eq!(offsets, &vec![0x28, 0x10]);
            }
            other => panic!("expected PointerChain, got {other:?}"),
        }

        // Write package and validate end-to-end against JSON Schema + SHA-256 package validator.
        result.package.write_to_dir(&out_dir).unwrap();
        let (validated, _) = validate_trainer_package_dir(&out_dir).unwrap();
        assert_eq!(validated.features.len(), 2);

        let _ = std::fs::remove_dir_all(&out_dir);
    }
}
