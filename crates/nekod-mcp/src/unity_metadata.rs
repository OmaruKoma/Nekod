//! Pure-Rust read-only ECMA-335 CLI metadata reader for Unity assemblies
//! (`*_Data/Managed/*.dll` for Mono and `BepInEx/interop/*.dll` for IL2CPP).
//!
//! Never loads or executes game binaries; inspects PE/CLR/BSJB metadata bytes
//! read-only to surface class hierarchies, fields, properties, and method signatures.

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;

const MAX_DLL_BYTES: u64 = 128 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UnityTypeSummary {
    pub full_name: String,
    pub base_type: Option<String>,
    pub is_public: bool,
    pub field_count: usize,
    pub property_count: usize,
    pub method_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UnityAssemblyOverview {
    pub assembly_path: String,
    pub available_assemblies: Vec<String>,
    pub total_types: usize,
    pub returned_types: usize,
    pub types: Vec<UnityTypeSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UnityFieldInfo {
    pub name: String,
    pub field_type: String,
    pub declaring_type: String,
    pub is_public: bool,
    pub is_static: bool,
    /// `true` when `NekodUnityBridge` (`BindingFlags.Public | NonPublic | FlattenHierarchy`)
    /// can access it directly on this type (C# reflection only reflects non-public base fields via declaring type).
    pub bridge_accessible: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UnityPropertyInfo {
    pub name: String,
    pub property_type: String,
    pub declaring_type: String,
    pub can_read: bool,
    pub can_write: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UnityMethodInfo {
    pub name: String,
    pub return_type: String,
    pub param_count: usize,
    pub declaring_type: String,
    pub is_public: bool,
    pub is_static: bool,
    pub invoke_method_compatible: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UnityTypeDetail {
    pub assembly_path: String,
    pub full_name: String,
    pub base_type: Option<String>,
    pub inheritance_chain: Vec<String>,
    pub subclasses: Vec<String>,
    pub fields: Vec<UnityFieldInfo>,
    pub properties: Vec<UnityPropertyInfo>,
    pub methods: Vec<UnityMethodInfo>,
}

/// Discovers candidate Unity managed/interop assemblies in a game install directory,
/// putting `Assembly-CSharp.dll` first when present.
pub fn discover_unity_assemblies(install_dir: &Path) -> Vec<PathBuf> {
    let mut dlls = Vec::new();
    let mut dirs = Vec::new();

    if let Ok(entries) = fs::read_dir(install_dir) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir()
                && p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.ends_with("_Data"))
            {
                let managed = p.join("Managed");
                if managed.is_dir() {
                    dirs.push(managed);
                }
            }
        }
    }

    let interop = install_dir.join("BepInEx").join("interop");
    if interop.is_dir() {
        dirs.push(interop);
    }

    for dir in dirs {
        if let Ok(entries) = fs::read_dir(dir) {
            for entry in entries.flatten() {
                let p = entry.path();
                if p.is_file()
                    && p.extension()
                        .and_then(|e| e.to_str())
                        .is_some_and(|e| e.eq_ignore_ascii_case("dll"))
                {
                    dlls.push(p);
                }
            }
        }
    }

    dlls.sort_by(|a, b| {
        let a_name = a.file_name().and_then(|n| n.to_str()).unwrap_or("");
        let b_name = b.file_name().and_then(|n| n.to_str()).unwrap_or("");
        let priority = |name: &str| {
            if name.eq_ignore_ascii_case("Assembly-CSharp.dll") {
                0
            } else if name.eq_ignore_ascii_case("Assembly-CSharp-firstpass.dll") {
                1
            } else {
                2
            }
        };
        priority(a_name).cmp(&priority(b_name)).then_with(|| {
            a_name
                .to_ascii_lowercase()
                .cmp(&b_name.to_ascii_lowercase())
        })
    });
    dlls
}

pub fn list_types_in_assembly(
    dll_path: &Path,
    available_assemblies: Vec<String>,
    filter: Option<&str>,
    limit: usize,
    include_compiler_generated: bool,
) -> Result<UnityAssemblyOverview, String> {
    let md = ParsedAssembly::from_file(dll_path)?;
    let filter_lower = filter
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_ascii_lowercase);
    let max_items = limit.clamp(1, 200);

    let mut matched = Vec::new();
    for (idx, td) in md.type_defs.iter().enumerate() {
        let full_name = md.type_def_full_name(idx + 1);
        if full_name == "<Module>" {
            continue;
        }
        if !include_compiler_generated && is_compiler_generated(&full_name) {
            continue;
        }
        if let Some(needle) = &filter_lower
            && !full_name.to_ascii_lowercase().contains(needle)
        {
            continue;
        }

        let td_idx = idx + 1;
        let field_count = md.field_range_for(td_idx).len();
        let method_count = md.method_range_for(td_idx).len();
        let property_count = md.property_range_for(td_idx).len();
        let vis = td.flags & 0x07;
        let is_public = vis == 0x01 || vis == 0x02;

        matched.push(UnityTypeSummary {
            full_name,
            base_type: md.resolve_type_def_or_ref(td.extends),
            is_public,
            field_count,
            property_count,
            method_count,
        });
    }

    matched.sort_by(|a, b| a.full_name.cmp(&b.full_name));
    let total_types = matched.len();
    matched.truncate(max_items);

    Ok(UnityAssemblyOverview {
        assembly_path: dll_path.display().to_string(),
        available_assemblies,
        total_types,
        returned_types: matched.len(),
        types: matched,
    })
}

pub fn inspect_type_in_assembly(
    dll_path: &Path,
    type_name: &str,
) -> Result<UnityTypeDetail, String> {
    let md = ParsedAssembly::from_file(dll_path)?;
    let target_name = type_name.trim();
    if target_name.is_empty() {
        return Err("type_name must be non-empty".to_owned());
    }

    let mut found_idx: Option<usize> = None;
    let mut short_match: Option<usize> = None;
    for idx in 0..md.type_defs.len() {
        let td_idx = idx + 1;
        let full = md.type_def_full_name(td_idx);
        if full == target_name {
            found_idx = Some(td_idx);
            break;
        }
        if short_match.is_none() && md.type_defs[idx].name == target_name {
            short_match = Some(td_idx);
        }
    }

    let target_td = found_idx
        .or(short_match)
        .ok_or_else(|| format!("type '{target_name}' not found in '{}'", dll_path.display()))?;

    let target_full_name = md.type_def_full_name(target_td);
    let base_type = md.resolve_type_def_or_ref(md.type_defs[target_td - 1].extends);

    // Direct subclasses inside this assembly.
    let mut subclasses = Vec::new();
    for idx in 0..md.type_defs.len() {
        let candidate = idx + 1;
        if md.extends_type_def(md.type_defs[idx].extends) == Some(target_td) {
            subclasses.push(md.type_def_full_name(candidate));
        }
    }
    subclasses.sort();

    // Walk intra-assembly TypeDef inheritance chain (cycle-guarded) and collect external tail.
    let mut chain_tds = Vec::new();
    let mut chain_names = Vec::new();
    let mut cur = Some(target_td);
    let mut steps = 0usize;
    while let Some(td_idx) = cur {
        if steps >= 32 || chain_tds.contains(&td_idx) {
            break;
        }
        steps += 1;
        chain_tds.push(td_idx);
        let full = md.type_def_full_name(td_idx);
        chain_names.push(full);
        let ext = md.type_defs[td_idx - 1].extends;
        if let Some(parent_td) = md.extends_type_def(ext) {
            cur = Some(parent_td);
        } else {
            if let Some(ext_name) = md.resolve_type_def_or_ref(ext) {
                chain_names.push(ext_name);
            }
            break;
        }
    }

    let mut fields = Vec::new();
    let mut properties = Vec::new();
    let mut methods = Vec::new();

    for td_idx in chain_tds {
        let decl_full = md.type_def_full_name(td_idx);
        let decl_short = md.type_defs[td_idx - 1].name.clone();

        for f_idx in md.field_range_for(td_idx) {
            let Some(f) = md.fields.get(f_idx - 1) else {
                continue;
            };
            let is_public = (f.flags & 0x0007) == 0x0006;
            let is_static = (f.flags & 0x0010) != 0;
            fields.push(UnityFieldInfo {
                name: f.name.clone(),
                field_type: md.decode_field_sig(f.signature),
                declaring_type: decl_full.clone(),
                is_public,
                is_static,
                bridge_accessible: is_public || decl_full == target_full_name,
            });
        }

        for p_idx in md.property_range_for(td_idx) {
            let Some(p) = md.properties.get(p_idx - 1) else {
                continue;
            };
            let sem = md.property_semantics.get(&p_idx).copied().unwrap_or(0);
            properties.push(UnityPropertyInfo {
                name: p.name.clone(),
                property_type: md.decode_property_sig(p.signature),
                declaring_type: decl_full.clone(),
                can_read: (sem & 0x0002) != 0,
                can_write: (sem & 0x0001) != 0,
            });
        }

        for m_idx in md.method_range_for(td_idx) {
            let Some(m) = md.methods.get(m_idx - 1) else {
                continue;
            };
            // Skip property/event special methods (`SpecialName = 0x0800`) and constructors unless explicitly inspected.
            if (m.flags & 0x0800) != 0 {
                continue;
            }
            let is_public = (m.flags & 0x0007) == 0x0006;
            let is_static = (m.flags & 0x0010) != 0;
            let (param_count, return_type) = md.decode_method_sig(m.signature);
            let _ = &decl_short;
            methods.push(UnityMethodInfo {
                name: m.name.clone(),
                return_type,
                param_count,
                declaring_type: decl_full.clone(),
                is_public,
                is_static,
                invoke_method_compatible: param_count == 0,
            });
        }
    }

    fields.sort_by(|a, b| a.name.cmp(&b.name));
    properties.sort_by(|a, b| a.name.cmp(&b.name));
    methods.sort_by(|a, b| {
        a.name
            .cmp(&b.name)
            .then_with(|| a.param_count.cmp(&b.param_count))
    });

    Ok(UnityTypeDetail {
        assembly_path: dll_path.display().to_string(),
        full_name: target_full_name,
        base_type,
        inheritance_chain: chain_names,
        subclasses,
        fields,
        properties,
        methods,
    })
}

fn is_compiler_generated(full_name: &str) -> bool {
    full_name.contains("<>") || full_name.contains(">d__") || full_name.starts_with('<')
}

// --- Low-level ECMA-335 PE/BSJB parser ---

struct Section {
    virtual_address: u32,
    virtual_size: u32,
    raw_size: u32,
    raw_ptr: u32,
}

#[derive(Clone)]
struct TypeRefRow {
    name: String,
    namespace: String,
}

#[derive(Clone)]
struct TypeDefRow {
    flags: u32,
    name: String,
    namespace: String,
    extends: u32,
    field_list: u32,
    method_list: u32,
}

#[derive(Clone)]
struct FieldRow {
    flags: u16,
    name: String,
    signature: u32,
}

#[derive(Clone)]
struct MethodRow {
    flags: u16,
    name: String,
    signature: u32,
}

#[derive(Clone)]
struct PropertyRow {
    name: String,
    signature: u32,
}

struct ParsedAssembly {
    blob: Vec<u8>,
    type_refs: Vec<TypeRefRow>,
    type_defs: Vec<TypeDefRow>,
    fields: Vec<FieldRow>,
    methods: Vec<MethodRow>,
    property_map: Vec<(usize, usize)>,
    properties: Vec<PropertyRow>,
    property_semantics: HashMap<usize, u16>,
    type_specs: Vec<u32>,
    enclosing_by_nested: HashMap<usize, usize>,
}

impl ParsedAssembly {
    fn from_file(path: &Path) -> Result<Self, String> {
        let meta = fs::metadata(path)
            .map_err(|e| format!("cannot access assembly '{}': {e}", path.display()))?;
        if !meta.is_file() {
            return Err(format!("'{}' is not a regular file", path.display()));
        }
        if meta.len() > MAX_DLL_BYTES {
            return Err(format!(
                "assembly '{}' exceeds 128 MiB limit",
                path.display()
            ));
        }
        let bytes = fs::read(path)
            .map_err(|e| format!("cannot read assembly '{}': {e}", path.display()))?;
        Self::parse(&bytes)
    }

    fn parse(data: &[u8]) -> Result<Self, String> {
        if data.len() < 0x40 || read_u16(data, 0)? != 0x5A4D {
            return Err("invalid PE file (missing MZ header)".to_owned());
        }
        let pe_off = read_u32(data, 0x3C)? as usize;
        if read_u32(data, pe_off)? != 0x0000_4550 {
            return Err("invalid PE signature".to_owned());
        }
        let num_sections = read_u16(data, pe_off + 6)? as usize;
        let opt_size = read_u16(data, pe_off + 20)? as usize;
        let opt_off = pe_off + 24;
        let magic = read_u16(data, opt_off)?;
        let clr_dir_off = match magic {
            0x010B => opt_off + 208,
            0x020B => opt_off + 224,
            other => {
                return Err(format!(
                    "unsupported PE optional header magic 0x{other:04x}"
                ));
            }
        };
        let clr_rva = read_u32(data, clr_dir_off)?;
        let clr_size = read_u32(data, clr_dir_off + 4)?;
        if clr_rva == 0 || clr_size < 16 {
            return Err("assembly contains no CLI (.NET) metadata header".to_owned());
        }

        let sec_off = opt_off + opt_size;
        let mut sections = Vec::with_capacity(num_sections);
        for i in 0..num_sections {
            let off = sec_off + i * 40;
            sections.push(Section {
                virtual_size: read_u32(data, off + 8)?,
                virtual_address: read_u32(data, off + 12)?,
                raw_size: read_u32(data, off + 16)?,
                raw_ptr: read_u32(data, off + 20)?,
            });
        }

        let clr_off = rva_to_offset(&sections, clr_rva, data.len())?;
        let md_rva = read_u32(data, clr_off + 8)?;
        if md_rva == 0 {
            return Err("CLI header contains null metadata RVA".to_owned());
        }
        let md_off = rva_to_offset(&sections, md_rva, data.len())?;
        if read_u32(data, md_off)? != 0x424A_5342 {
            return Err("invalid CLI metadata root (missing BSJB signature)".to_owned());
        }

        let ver_len = read_u32(data, md_off + 12)? as usize;
        let mut cursor = md_off + 16 + ((ver_len + 3) & !3);
        cursor += 2; // Flags
        let num_streams = read_u16(data, cursor)? as usize;
        cursor += 2;

        let mut strings: Option<&[u8]> = None;
        let mut blob: Option<&[u8]> = None;
        let mut tables_stream: Option<&[u8]> = None;

        for _ in 0..num_streams {
            let stream_rel = read_u32(data, cursor)? as usize;
            let stream_size = read_u32(data, cursor + 4)? as usize;
            cursor += 8;

            let name_start = cursor;
            while cursor < data.len() && data[cursor] != 0 {
                cursor += 1;
            }
            let name = std::str::from_utf8(&data[name_start..cursor]).unwrap_or("");
            cursor += 1; // NUL
            let padded_name_len = ((cursor - name_start) + 3) & !3;
            cursor = name_start + padded_name_len;

            let s_off = md_off
                .checked_add(stream_rel)
                .ok_or_else(|| "metadata stream offset overflow".to_owned())?;
            let s_end = s_off
                .checked_add(stream_size)
                .filter(|&e| e <= data.len())
                .ok_or_else(|| format!("metadata stream '{name}' out of bounds"))?;
            let slice = &data[s_off..s_end];

            match name {
                "#Strings" => strings = Some(slice),
                "#Blob" => blob = Some(slice),
                "#~" | "#-" => tables_stream = Some(slice),
                _ => {}
            }
        }

        let strings = strings.unwrap_or(&[]).to_vec();
        let blob = blob.unwrap_or(&[]).to_vec();
        let ts = tables_stream.ok_or_else(|| "missing #~ metadata tables stream".to_owned())?;
        Self::parse_tables(ts, strings, blob)
    }

    fn parse_tables(ts: &[u8], strings: Vec<u8>, blob: Vec<u8>) -> Result<Self, String> {
        if ts.len() < 24 {
            return Err("truncated #~ tables stream".to_owned());
        }
        let heap_sizes = ts[6];
        let str_idx = if (heap_sizes & 0x01) != 0 { 4 } else { 2 };
        let guid_idx = if (heap_sizes & 0x02) != 0 { 4 } else { 2 };
        let blob_idx = if (heap_sizes & 0x04) != 0 { 4 } else { 2 };

        let valid = read_u64(ts, 8)?;
        let mut row_counts = [0usize; 64];
        let mut pos = 24usize;
        for (tbl, count_slot) in row_counts.iter_mut().enumerate() {
            if (valid & (1u64 << tbl)) != 0 {
                *count_slot = read_u32(ts, pos)? as usize;
                pos += 4;
            }
        }
        if (heap_sizes & 0x40) != 0 {
            pos += 4;
        }

        let simple = |tbl: usize| -> usize {
            if row_counts.get(tbl).copied().unwrap_or(0) < 65_536 {
                2
            } else {
                4
            }
        };
        let coded = |bits: u32, tables: &[usize]| -> usize {
            let max_rows = tables
                .iter()
                .filter_map(|&t| row_counts.get(t).copied())
                .max()
                .unwrap_or(0);
            if max_rows < (1usize << (16 - bits)) {
                2
            } else {
                4
            }
        };

        let type_def_or_ref = coded(2, &[0x02, 0x01, 0x1B]);
        let has_constant = coded(2, &[0x04, 0x08, 0x17]);
        let has_custom_attr = coded(
            5,
            &[
                0x06, 0x04, 0x01, 0x02, 0x08, 0x09, 0x0A, 0x00, 0x0E, 0x17, 0x14, 0x11, 0x1A, 0x1B,
                0x20, 0x23, 0x26, 0x27, 0x28, 0x2A, 0x2B, 0x2C,
            ],
        );
        let has_field_marshal = coded(1, &[0x04, 0x08]);
        let has_decl_security = coded(2, &[0x02, 0x06, 0x20]);
        let member_ref_parent = coded(3, &[0x02, 0x01, 0x1A, 0x06, 0x1B]);
        let has_semantics = coded(1, &[0x14, 0x17]);
        let method_def_or_ref = coded(1, &[0x06, 0x0A]);
        let member_forwarded = coded(1, &[0x04, 0x06]);
        let implementation = coded(2, &[0x26, 0x23, 0x27]);
        let custom_attr_type = coded(3, &[0x06, 0x0A]);
        let resolution_scope = coded(2, &[0x00, 0x1A, 0x23, 0x01]);
        let type_or_method_def = coded(1, &[0x02, 0x06]);

        let mut type_refs = Vec::new();
        let mut type_defs = Vec::new();
        let mut fields = Vec::new();
        let mut methods = Vec::new();
        let mut property_map = Vec::new();
        let mut properties = Vec::new();
        let mut property_semantics = HashMap::new();
        let mut type_specs = Vec::new();
        let mut enclosing_by_nested = HashMap::new();

        for (tbl, &count) in row_counts.iter().enumerate() {
            if (valid & (1u64 << tbl)) == 0 {
                continue;
            }
            let row_width = match tbl {
                0x00 => 2 + str_idx + guid_idx * 3,
                0x01 => resolution_scope + str_idx * 2,
                0x02 => 4 + str_idx * 2 + type_def_or_ref + simple(0x04) + simple(0x06),
                0x03 => simple(0x04),
                0x04 => 2 + str_idx + blob_idx,
                0x05 => simple(0x06),
                0x06 => 4 + 2 + 2 + str_idx + blob_idx + simple(0x08),
                0x07 => simple(0x08),
                0x08 => 2 + 2 + str_idx,
                0x09 => simple(0x02) + type_def_or_ref,
                0x0A => member_ref_parent + str_idx + blob_idx,
                0x0B => 2 + has_constant + blob_idx,
                0x0C => has_custom_attr + custom_attr_type + blob_idx,
                0x0D => has_field_marshal + blob_idx,
                0x0E => 2 + has_decl_security + blob_idx,
                0x0F => 2 + 4 + simple(0x02),
                0x10 => 4 + simple(0x04),
                0x11 => blob_idx,
                0x12 => simple(0x02) + simple(0x14),
                0x13 => simple(0x14),
                0x14 => 2 + str_idx + type_def_or_ref,
                0x15 => simple(0x02) + simple(0x17),
                0x16 => simple(0x17),
                0x17 => 2 + str_idx + blob_idx,
                0x18 => 2 + simple(0x06) + has_semantics,
                0x19 => simple(0x02) + method_def_or_ref * 2,
                0x1A => str_idx,
                0x1B => blob_idx,
                0x1C => 2 + member_forwarded + str_idx + simple(0x1A),
                0x1D => 4 + simple(0x04),
                0x1E => 4 + 4,
                0x1F => 4,
                0x20 => 16 + blob_idx + str_idx * 2,
                0x21 => 4,
                0x22 => 12,
                0x23 => 12 + blob_idx * 2 + str_idx * 2,
                0x24 => 4 + simple(0x23),
                0x25 => 12 + simple(0x23),
                0x26 => 4 + str_idx + blob_idx,
                0x27 => 4 + 4 + str_idx * 2 + implementation,
                0x28 => 4 + 4 + str_idx + implementation,
                0x29 => simple(0x02) * 2,
                0x2A => 2 + 2 + type_or_method_def + str_idx,
                0x2B => method_def_or_ref + blob_idx,
                0x2C => simple(0x2A) + type_def_or_ref,
                other => return Err(format!("unsupported ECMA-335 table id 0x{other:02x}")),
            };

            let end_pos = pos
                .checked_add(row_width.saturating_mul(count))
                .filter(|&e| e <= ts.len())
                .ok_or_else(|| format!("ECMA-335 table 0x{tbl:02x} exceeds stream bounds"))?;

            for r in 0..count {
                let row = &ts[pos + r * row_width..pos + (r + 1) * row_width];
                match tbl {
                    0x01 => {
                        let name_off = read_idx(row, resolution_scope, str_idx);
                        let ns_off = read_idx(row, resolution_scope + str_idx, str_idx);
                        type_refs.push(TypeRefRow {
                            name: read_cstr(&strings, name_off),
                            namespace: read_cstr(&strings, ns_off),
                        });
                    }
                    0x02 => {
                        let flags = read_u32(row, 0)?;
                        let name_off = read_idx(row, 4, str_idx);
                        let ns_off = read_idx(row, 4 + str_idx, str_idx);
                        let extends = read_idx(row, 4 + str_idx * 2, type_def_or_ref);
                        let field_list =
                            read_idx(row, 4 + str_idx * 2 + type_def_or_ref, simple(0x04));
                        let method_list = read_idx(
                            row,
                            4 + str_idx * 2 + type_def_or_ref + simple(0x04),
                            simple(0x06),
                        );
                        type_defs.push(TypeDefRow {
                            flags,
                            name: read_cstr(&strings, name_off),
                            namespace: read_cstr(&strings, ns_off),
                            extends,
                            field_list,
                            method_list,
                        });
                    }
                    0x04 => {
                        let flags = read_u16(row, 0)?;
                        let name_off = read_idx(row, 2, str_idx);
                        let sig = read_idx(row, 2 + str_idx, blob_idx);
                        fields.push(FieldRow {
                            flags,
                            name: read_cstr(&strings, name_off),
                            signature: sig,
                        });
                    }
                    0x06 => {
                        let flags = read_u16(row, 6)?;
                        let name_off = read_idx(row, 8, str_idx);
                        let sig = read_idx(row, 8 + str_idx, blob_idx);
                        methods.push(MethodRow {
                            flags,
                            name: read_cstr(&strings, name_off),
                            signature: sig,
                        });
                    }
                    0x15 => {
                        let parent = read_idx(row, 0, simple(0x02)) as usize;
                        let prop_list = read_idx(row, simple(0x02), simple(0x17)) as usize;
                        property_map.push((parent, prop_list));
                    }
                    0x17 => {
                        let name_off = read_idx(row, 2, str_idx);
                        let sig = read_idx(row, 2 + str_idx, blob_idx);
                        properties.push(PropertyRow {
                            name: read_cstr(&strings, name_off),
                            signature: sig,
                        });
                    }
                    0x18 => {
                        let sem = read_u16(row, 0)?;
                        let assoc = read_idx(row, 2 + simple(0x06), has_semantics);
                        // HasSemantics tag bit 0: 0 = Event, 1 = Property.
                        if (assoc & 1) == 1 {
                            let prop_idx = (assoc >> 1) as usize;
                            if prop_idx != 0 {
                                property_semantics
                                    .entry(prop_idx)
                                    .and_modify(|v| *v |= sem)
                                    .or_insert(sem);
                            }
                        }
                    }
                    0x1B => {
                        type_specs.push(read_idx(row, 0, blob_idx));
                    }
                    0x29 => {
                        let nested = read_idx(row, 0, simple(0x02)) as usize;
                        let enclosing = read_idx(row, simple(0x02), simple(0x02)) as usize;
                        if nested != 0 && enclosing != 0 {
                            enclosing_by_nested.insert(nested, enclosing);
                        }
                    }
                    _ => {}
                }
            }
            pos = end_pos;
        }

        property_map.sort_by_key(|(parent, _)| *parent);

        Ok(Self {
            blob,
            type_refs,
            type_defs,
            fields,
            methods,
            property_map,
            properties,
            property_semantics,
            type_specs,
            enclosing_by_nested,
        })
    }

    fn type_def_full_name(&self, td_idx: usize) -> String {
        self.type_def_full_name_inner(td_idx, 0)
    }

    fn type_def_full_name_inner(&self, td_idx: usize, depth: usize) -> String {
        let Some(td) = td_idx.checked_sub(1).and_then(|i| self.type_defs.get(i)) else {
            return format!("TypeDef#{td_idx}");
        };
        if depth < 16
            && let Some(&enclosing) = self.enclosing_by_nested.get(&td_idx)
        {
            let outer = self.type_def_full_name_inner(enclosing, depth + 1);
            return format!("{outer}+{}", td.name);
        }
        if td.namespace.is_empty() {
            td.name.clone()
        } else {
            format!("{}.{}", td.namespace, td.name)
        }
    }

    fn type_ref_full_name(&self, tr_idx: usize) -> Option<String> {
        let tr = tr_idx.checked_sub(1).and_then(|i| self.type_refs.get(i))?;
        Some(if tr.namespace.is_empty() {
            tr.name.clone()
        } else {
            format!("{}.{}", tr.namespace, tr.name)
        })
    }

    fn extends_type_def(&self, coded: u32) -> Option<usize> {
        if coded == 0 {
            return None;
        }
        // TypeDefOrRef tag 0 => TypeDef
        if (coded & 0x03) == 0 {
            let idx = (coded >> 2) as usize;
            if idx >= 1 && idx <= self.type_defs.len() {
                return Some(idx);
            }
        }
        None
    }

    fn resolve_type_def_or_ref(&self, coded: u32) -> Option<String> {
        if coded == 0 {
            return None;
        }
        let tag = coded & 0x03;
        let idx = (coded >> 2) as usize;
        if idx == 0 {
            return None;
        }
        match tag {
            0 => Some(self.type_def_full_name(idx)),
            1 => self.type_ref_full_name(idx),
            2 => {
                let blob_off = *self.type_specs.get(idx - 1)?;
                let sig = self.blob_slice(blob_off)?;
                let mut c = 0usize;
                Some(self.decode_type_sig(sig, &mut c, 0))
            }
            _ => None,
        }
    }

    fn field_range_for(&self, td_idx: usize) -> std::ops::Range<usize> {
        let Some(td) = td_idx.checked_sub(1).and_then(|i| self.type_defs.get(i)) else {
            return 1..1;
        };
        let start = td.field_list as usize;
        let end = self
            .type_defs
            .get(td_idx)
            .map(|next| next.field_list as usize)
            .unwrap_or(self.fields.len() + 1);
        if start == 0 || end < start || start > self.fields.len() + 1 {
            return 1..1;
        }
        start..end.min(self.fields.len() + 1)
    }

    fn method_range_for(&self, td_idx: usize) -> std::ops::Range<usize> {
        let Some(td) = td_idx.checked_sub(1).and_then(|i| self.type_defs.get(i)) else {
            return 1..1;
        };
        let start = td.method_list as usize;
        let end = self
            .type_defs
            .get(td_idx)
            .map(|next| next.method_list as usize)
            .unwrap_or(self.methods.len() + 1);
        if start == 0 || end < start || start > self.methods.len() + 1 {
            return 1..1;
        }
        start..end.min(self.methods.len() + 1)
    }

    fn property_range_for(&self, td_idx: usize) -> std::ops::Range<usize> {
        let Ok(pos) = self.property_map.binary_search_by_key(&td_idx, |(p, _)| *p) else {
            return 1..1;
        };
        let start = self.property_map[pos].1;
        let end = self
            .property_map
            .get(pos + 1)
            .map(|(_, nxt)| *nxt)
            .unwrap_or(self.properties.len() + 1);
        if start == 0 || end < start || start > self.properties.len() + 1 {
            return 1..1;
        }
        start..end.min(self.properties.len() + 1)
    }

    fn blob_slice(&self, offset: u32) -> Option<&[u8]> {
        let off = offset as usize;
        if off >= self.blob.len() {
            return None;
        }
        let mut c = off;
        let len = decode_compressed_uint(&self.blob, &mut c)? as usize;
        self.blob.get(c..c.saturating_add(len))
    }

    fn decode_field_sig(&self, blob_off: u32) -> String {
        let Some(sig) = self.blob_slice(blob_off) else {
            return "Unknown".to_owned();
        };
        if sig.first() != Some(&0x06) {
            return "Unknown".to_owned();
        }
        let mut c = 1usize;
        self.decode_type_sig(sig, &mut c, 0)
    }

    fn decode_property_sig(&self, blob_off: u32) -> String {
        let Some(sig) = self.blob_slice(blob_off) else {
            return "Unknown".to_owned();
        };
        if sig.is_empty() || (sig[0] & 0x0F) != 0x08 {
            return "Unknown".to_owned();
        }
        let mut c = 1usize;
        if decode_compressed_uint(sig, &mut c).is_none() {
            return "Unknown".to_owned();
        }
        self.decode_type_sig(sig, &mut c, 0)
    }

    fn decode_method_sig(&self, blob_off: u32) -> (usize, String) {
        let Some(sig) = self.blob_slice(blob_off) else {
            return (0, "Unknown".to_owned());
        };
        if sig.is_empty() {
            return (0, "Unknown".to_owned());
        }
        let cc = sig[0];
        let mut c = 1usize;
        if (cc & 0x10) != 0 && decode_compressed_uint(sig, &mut c).is_none() {
            return (0, "Unknown".to_owned());
        }
        let param_count = decode_compressed_uint(sig, &mut c).unwrap_or(0) as usize;
        let ret = self.decode_type_sig(sig, &mut c, 0);
        (param_count, ret)
    }

    fn decode_type_sig(&self, sig: &[u8], c: &mut usize, depth: usize) -> String {
        if depth > 12 || *c >= sig.len() {
            return "Unknown".to_owned();
        }
        let elem = sig[*c];
        *c += 1;
        match elem {
            0x01 => "Void".to_owned(),
            0x02 => "Boolean".to_owned(),
            0x03 => "Char".to_owned(),
            0x04 => "SByte".to_owned(),
            0x05 => "Byte".to_owned(),
            0x06 => "Int16".to_owned(),
            0x07 => "UInt16".to_owned(),
            0x08 => "Int32".to_owned(),
            0x09 => "UInt32".to_owned(),
            0x0A => "Int64".to_owned(),
            0x0B => "UInt64".to_owned(),
            0x0C => "Single".to_owned(),
            0x0D => "Double".to_owned(),
            0x0E => "String".to_owned(),
            0x0F => format!("{}*", self.decode_type_sig(sig, c, depth + 1)),
            0x10 => format!("{}&", self.decode_type_sig(sig, c, depth + 1)),
            0x11 | 0x12 => {
                let coded = decode_compressed_uint(sig, c).unwrap_or(0);
                self.resolve_type_def_or_ref(coded)
                    .unwrap_or_else(|| "Object".to_owned())
            }
            0x13 => {
                let idx = decode_compressed_uint(sig, c).unwrap_or(0);
                format!("T{idx}")
            }
            0x14 => {
                let inner = self.decode_type_sig(sig, c, depth + 1);
                let rank = decode_compressed_uint(sig, c).unwrap_or(1) as usize;
                let num_sizes = decode_compressed_uint(sig, c).unwrap_or(0) as usize;
                for _ in 0..num_sizes {
                    let _ = decode_compressed_uint(sig, c);
                }
                let num_lo = decode_compressed_uint(sig, c).unwrap_or(0) as usize;
                for _ in 0..num_lo {
                    let _ = decode_compressed_uint(sig, c);
                }
                format!("{inner}[{}]", ",".repeat(rank.saturating_sub(1)))
            }
            0x15 => {
                let base = self.decode_type_sig(sig, c, depth + 1);
                let arg_cnt = decode_compressed_uint(sig, c).unwrap_or(0) as usize;
                let mut args = Vec::with_capacity(arg_cnt.min(16));
                for _ in 0..arg_cnt.min(16) {
                    args.push(self.decode_type_sig(sig, c, depth + 1));
                }
                format!("{base}<{}>", args.join(", "))
            }
            0x18 => "IntPtr".to_owned(),
            0x19 => "UIntPtr".to_owned(),
            0x1C => "Object".to_owned(),
            0x1D => format!("{}[]", self.decode_type_sig(sig, c, depth + 1)),
            0x1E => {
                let idx = decode_compressed_uint(sig, c).unwrap_or(0);
                format!("M{idx}")
            }
            0x1F | 0x20 => {
                let _ = decode_compressed_uint(sig, c);
                self.decode_type_sig(sig, c, depth + 1)
            }
            0x45 => self.decode_type_sig(sig, c, depth + 1), // PINNED
            other => format!("Type(0x{other:02x})"),
        }
    }
}

fn rva_to_offset(sections: &[Section], rva: u32, file_len: usize) -> Result<usize, String> {
    for sec in sections {
        let span = sec.virtual_size.max(sec.raw_size);
        if rva >= sec.virtual_address && rva < sec.virtual_address.saturating_add(span) {
            let off = sec.raw_ptr.saturating_add(rva - sec.virtual_address) as usize;
            if off < file_len {
                return Ok(off);
            }
        }
    }
    Err(format!("RVA 0x{rva:08x} does not map into any PE section"))
}

fn decode_compressed_uint(buf: &[u8], c: &mut usize) -> Option<u32> {
    let b0 = *buf.get(*c)?;
    if (b0 & 0x80) == 0 {
        *c += 1;
        Some(u32::from(b0))
    } else if (b0 & 0xC0) == 0x80 {
        let b1 = *buf.get(*c + 1)?;
        *c += 2;
        Some((u32::from(b0 & 0x3F) << 8) | u32::from(b1))
    } else if (b0 & 0xE0) == 0xC0 {
        let b1 = *buf.get(*c + 1)?;
        let b2 = *buf.get(*c + 2)?;
        let b3 = *buf.get(*c + 3)?;
        *c += 4;
        Some(
            (u32::from(b0 & 0x1F) << 24)
                | (u32::from(b1) << 16)
                | (u32::from(b2) << 8)
                | u32::from(b3),
        )
    } else {
        None
    }
}

fn read_idx(row: &[u8], off: usize, size: usize) -> u32 {
    if size == 4 {
        u32::from_le_bytes(row[off..off + 4].try_into().unwrap_or([0; 4]))
    } else {
        u32::from(u16::from_le_bytes(
            row[off..off + 2].try_into().unwrap_or([0; 2]),
        ))
    }
}

fn read_cstr(heap: &[u8], off: u32) -> String {
    let start = off as usize;
    if start >= heap.len() {
        return String::new();
    }
    let end = heap[start..]
        .iter()
        .position(|&b| b == 0)
        .map(|rel| start + rel)
        .unwrap_or(heap.len());
    String::from_utf8_lossy(&heap[start..end]).into_owned()
}

fn read_u16(buf: &[u8], off: usize) -> Result<u16, String> {
    buf.get(off..off + 2)
        .and_then(|s| s.try_into().ok())
        .map(u16::from_le_bytes)
        .ok_or_else(|| format!("unexpected EOF reading u16 at 0x{off:x}"))
}

fn read_u32(buf: &[u8], off: usize) -> Result<u32, String> {
    buf.get(off..off + 4)
        .and_then(|s| s.try_into().ok())
        .map(u32::from_le_bytes)
        .ok_or_else(|| format!("unexpected EOF reading u32 at 0x{off:x}"))
}

fn read_u64(buf: &[u8], off: usize) -> Result<u64, String> {
    buf.get(off..off + 8)
        .and_then(|s| s.try_into().ok())
        .map(u64::from_le_bytes)
        .ok_or_else(|| format!("unexpected EOF reading u64 at 0x{off:x}"))
}

/// Helper used by tests and diagnostics to construct a minimal valid synthetic PE32 + CLI
/// metadata `.dll` image without running a C# compiler.
pub fn build_synthetic_unity_dll_for_test() -> Vec<u8> {
    let mut strings = vec![0u8];
    let mut str_offsets = BTreeMap::new();
    let mut intern = |s: &str| -> u16 {
        if let Some(&off) = str_offsets.get(s) {
            return off;
        }
        let off = strings.len() as u16;
        strings.extend_from_slice(s.as_bytes());
        strings.push(0);
        str_offsets.insert(s.to_owned(), off);
        off
    };

    let s_module = intern("<Module>");
    let s_unity_ns = intern("UnityEngine");
    let s_mono = intern("MonoBehaviour");
    let s_demo_ns = intern("Demo");
    let s_base = intern("BaseHealth");
    let s_player = intern("PlayerController");
    let s_immortal = intern("isImmortal");
    let s_stamina_rec = intern("staminaRecovery");
    let s_ammo_prop = intern("AllAmmoInfinity");
    let s_ammo_backing = intern("<AllAmmoInfinity>k__BackingField");
    let s_reset_health = intern("ResetHealth");
    let s_get_ammo = intern("get_AllAmmoInfinity");
    let s_set_ammo = intern("set_AllAmmoInfinity");

    let mut blob = vec![0u8];
    let mut add_blob = |bytes: &[u8]| -> u16 {
        let off = blob.len() as u16;
        blob.push(bytes.len() as u8);
        blob.extend_from_slice(bytes);
        off
    };
    let b_field_bool = add_blob(&[0x06, 0x02]); // FIELD Boolean
    let b_field_single = add_blob(&[0x06, 0x0C]); // FIELD Single
    let b_prop_bool = add_blob(&[0x28, 0x00, 0x02]); // PROPERTY+HASTHIS, 0 params, Boolean
    let b_meth_void0 = add_blob(&[0x20, 0x00, 0x01]); // HASTHIS, 0 params, Void
    let b_meth_bool0 = add_blob(&[0x20, 0x00, 0x02]); // HASTHIS, 0 params, Boolean
    let b_meth_set_bool = add_blob(&[0x20, 0x01, 0x01, 0x02]); // HASTHIS, 1 param (Boolean), Void

    // Align heaps to 4 bytes
    while strings.len() % 4 != 0 {
        strings.push(0);
    }
    while blob.len() % 4 != 0 {
        blob.push(0);
    }

    // Build #~ tables:
    // TypeRef(0x01): 1 row
    // TypeDef(0x02): 3 rows (<Module>, BaseHealth, PlayerController)
    // Field(0x04): 3 rows
    // MethodDef(0x06): 3 rows
    // PropertyMap(0x15): 1 row
    // Property(0x17): 1 row
    // MethodSemantics(0x18): 2 rows
    let valid: u64 = (1u64 << 0x01)
        | (1u64 << 0x02)
        | (1u64 << 0x04)
        | (1u64 << 0x06)
        | (1u64 << 0x15)
        | (1u64 << 0x17)
        | (1u64 << 0x18);

    let mut ts = Vec::new();
    ts.extend_from_slice(&0u32.to_le_bytes()); // Reserved
    ts.push(2); // Major
    ts.push(0); // Minor
    ts.push(0); // HeapSizes (all 2-byte indices)
    ts.push(1); // Reserved
    ts.extend_from_slice(&valid.to_le_bytes());
    ts.extend_from_slice(&0u64.to_le_bytes()); // Sorted
    for cnt in [1u32, 3, 3, 3, 1, 1, 2] {
        ts.extend_from_slice(&cnt.to_le_bytes());
    }

    // TypeRef[1]: UnityEngine.MonoBehaviour
    ts.extend_from_slice(&0u16.to_le_bytes()); // ResolutionScope
    ts.extend_from_slice(&s_mono.to_le_bytes());
    ts.extend_from_slice(&s_unity_ns.to_le_bytes());

    // TypeDef[1]: <Module>
    ts.extend_from_slice(&0u32.to_le_bytes());
    ts.extend_from_slice(&s_module.to_le_bytes());
    ts.extend_from_slice(&0u16.to_le_bytes());
    ts.extend_from_slice(&0u16.to_le_bytes()); // Extends null
    ts.extend_from_slice(&1u16.to_le_bytes()); // FieldList
    ts.extend_from_slice(&1u16.to_le_bytes()); // MethodList

    // TypeDef[2]: Demo.BaseHealth extends TypeRef[1] ((1 << 2) | 1 = 5)
    ts.extend_from_slice(&0x0000_0001u32.to_le_bytes()); // Public
    ts.extend_from_slice(&s_base.to_le_bytes());
    ts.extend_from_slice(&s_demo_ns.to_le_bytes());
    ts.extend_from_slice(&5u16.to_le_bytes());
    ts.extend_from_slice(&1u16.to_le_bytes()); // FieldList 1..2
    ts.extend_from_slice(&1u16.to_le_bytes()); // MethodList 1..2

    // TypeDef[3]: Demo.PlayerController extends TypeDef[2] ((2 << 2) | 0 = 8)
    ts.extend_from_slice(&0x0000_0001u32.to_le_bytes()); // Public
    ts.extend_from_slice(&s_player.to_le_bytes());
    ts.extend_from_slice(&s_demo_ns.to_le_bytes());
    ts.extend_from_slice(&8u16.to_le_bytes());
    ts.extend_from_slice(&2u16.to_le_bytes()); // FieldList 2..=3
    ts.extend_from_slice(&2u16.to_le_bytes()); // MethodList 2..=3

    // Field[1]: public Boolean isImmortal
    ts.extend_from_slice(&0x0006u16.to_le_bytes());
    ts.extend_from_slice(&s_immortal.to_le_bytes());
    ts.extend_from_slice(&b_field_bool.to_le_bytes());
    // Field[2]: public Single staminaRecovery
    ts.extend_from_slice(&0x0006u16.to_le_bytes());
    ts.extend_from_slice(&s_stamina_rec.to_le_bytes());
    ts.extend_from_slice(&b_field_single.to_le_bytes());
    // Field[3]: private Boolean <AllAmmoInfinity>k__BackingField
    ts.extend_from_slice(&0x0001u16.to_le_bytes());
    ts.extend_from_slice(&s_ammo_backing.to_le_bytes());
    ts.extend_from_slice(&b_field_bool.to_le_bytes());

    // MethodDef[1]: public Void ResetHealth()
    ts.extend_from_slice(&0u32.to_le_bytes());
    ts.extend_from_slice(&0u16.to_le_bytes());
    ts.extend_from_slice(&0x0006u16.to_le_bytes());
    ts.extend_from_slice(&s_reset_health.to_le_bytes());
    ts.extend_from_slice(&b_meth_void0.to_le_bytes());
    ts.extend_from_slice(&1u16.to_le_bytes());

    // MethodDef[2]: public specialname Boolean get_AllAmmoInfinity()
    ts.extend_from_slice(&0u32.to_le_bytes());
    ts.extend_from_slice(&0u16.to_le_bytes());
    ts.extend_from_slice(&0x0806u16.to_le_bytes());
    ts.extend_from_slice(&s_get_ammo.to_le_bytes());
    ts.extend_from_slice(&b_meth_bool0.to_le_bytes());
    ts.extend_from_slice(&1u16.to_le_bytes());

    // MethodDef[3]: public specialname Void set_AllAmmoInfinity(Boolean)
    ts.extend_from_slice(&0u32.to_le_bytes());
    ts.extend_from_slice(&0u16.to_le_bytes());
    ts.extend_from_slice(&0x0806u16.to_le_bytes());
    ts.extend_from_slice(&s_set_ammo.to_le_bytes());
    ts.extend_from_slice(&b_meth_set_bool.to_le_bytes());
    ts.extend_from_slice(&1u16.to_le_bytes());

    // PropertyMap[1]: TypeDef[3] -> Property[1]
    ts.extend_from_slice(&3u16.to_le_bytes());
    ts.extend_from_slice(&1u16.to_le_bytes());

    // Property[1]: Boolean AllAmmoInfinity
    ts.extend_from_slice(&0u16.to_le_bytes());
    ts.extend_from_slice(&s_ammo_prop.to_le_bytes());
    ts.extend_from_slice(&b_prop_bool.to_le_bytes());

    // MethodSemantics: Getter(2) -> MethodDef[2] + Property[1] ((1<<1)|1 = 3)
    ts.extend_from_slice(&0x0002u16.to_le_bytes());
    ts.extend_from_slice(&2u16.to_le_bytes());
    ts.extend_from_slice(&3u16.to_le_bytes());
    // MethodSemantics: Setter(1) -> MethodDef[3] + Property[1]
    ts.extend_from_slice(&0x0001u16.to_le_bytes());
    ts.extend_from_slice(&3u16.to_le_bytes());
    ts.extend_from_slice(&3u16.to_le_bytes());

    while ts.len() % 4 != 0 {
        ts.push(0);
    }

    // Build BSJB metadata block
    let ver = b"v4.0.30319\0\0"; // 12 bytes (multiple of 4)
    // Stream headers: "#~\0\0" (12B), "#Strings\0\0\0\0" (20B), "#Blob\0\0\0" (16B) = 48B
    let md_header_len = 16 + ver.len() + 4 + (8 + 4) + (8 + 12) + (8 + 8);
    let ts_off = md_header_len;
    let str_off = ts_off + ts.len();
    let blob_off = str_off + strings.len();

    let mut md = Vec::new();
    md.extend_from_slice(&0x424A_5342u32.to_le_bytes());
    md.extend_from_slice(&1u16.to_le_bytes());
    md.extend_from_slice(&1u16.to_le_bytes());
    md.extend_from_slice(&0u32.to_le_bytes());
    md.extend_from_slice(&(ver.len() as u32).to_le_bytes());
    md.extend_from_slice(ver);
    md.extend_from_slice(&0u16.to_le_bytes());
    md.extend_from_slice(&3u16.to_le_bytes());

    md.extend_from_slice(&(ts_off as u32).to_le_bytes());
    md.extend_from_slice(&(ts.len() as u32).to_le_bytes());
    md.extend_from_slice(b"#~\0\0");

    md.extend_from_slice(&(str_off as u32).to_le_bytes());
    md.extend_from_slice(&(strings.len() as u32).to_le_bytes());
    md.extend_from_slice(b"#Strings\0\0\0\0");

    md.extend_from_slice(&(blob_off as u32).to_le_bytes());
    md.extend_from_slice(&(blob.len() as u32).to_le_bytes());
    md.extend_from_slice(b"#Blob\0\0\0");

    assert_eq!(md.len(), md_header_len);
    md.extend_from_slice(&ts);
    md.extend_from_slice(&strings);
    md.extend_from_slice(&blob);

    // Wrap into minimal PE32 with .text section at RVA 0x2000, Raw 0x200
    let text_rva: u32 = 0x2000;
    let text_raw: u32 = 0x200;
    let clr_size: u32 = 72;
    let md_rva: u32 = text_rva + clr_size;

    let mut text = vec![0u8; clr_size as usize];
    text[0..4].copy_from_slice(&72u32.to_le_bytes()); // cb
    text[4..6].copy_from_slice(&2u16.to_le_bytes()); // MajorRuntimeVersion
    text[6..8].copy_from_slice(&5u16.to_le_bytes()); // MinorRuntimeVersion
    text[8..12].copy_from_slice(&md_rva.to_le_bytes()); // MetaData RVA
    text[12..16].copy_from_slice(&(md.len() as u32).to_le_bytes()); // MetaData Size
    text.extend_from_slice(&md);

    let mut pe = vec![0u8; text_raw as usize + text.len()];
    pe[0..2].copy_from_slice(&0x5A4Du16.to_le_bytes());
    let pe_off = 0x80usize;
    pe[0x3C..0x40].copy_from_slice(&(pe_off as u32).to_le_bytes());
    pe[pe_off..pe_off + 4].copy_from_slice(&0x0000_4550u32.to_le_bytes());
    pe[pe_off + 6..pe_off + 8].copy_from_slice(&1u16.to_le_bytes()); // 1 section
    let opt_size: u16 = 224;
    pe[pe_off + 20..pe_off + 22].copy_from_slice(&opt_size.to_le_bytes());
    let opt_off = pe_off + 24;
    pe[opt_off..opt_off + 2].copy_from_slice(&0x010Bu16.to_le_bytes()); // PE32
    pe[opt_off + 208..opt_off + 212].copy_from_slice(&text_rva.to_le_bytes()); // CLR RVA
    pe[opt_off + 212..opt_off + 216].copy_from_slice(&clr_size.to_le_bytes()); // CLR Size

    let sec_off = opt_off + opt_size as usize;
    pe[sec_off..sec_off + 6].copy_from_slice(b".text\0");
    pe[sec_off + 8..sec_off + 12].copy_from_slice(&(text.len() as u32).to_le_bytes());
    pe[sec_off + 12..sec_off + 16].copy_from_slice(&text_rva.to_le_bytes());
    pe[sec_off + 16..sec_off + 20].copy_from_slice(&(text.len() as u32).to_le_bytes());
    pe[sec_off + 20..sec_off + 24].copy_from_slice(&text_raw.to_le_bytes());

    pe[text_raw as usize..].copy_from_slice(&text);
    pe
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    #[test]
    fn parses_synthetic_unity_assembly_inheritance_fields_properties_and_methods() {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("nekod_ecmatest_{nanos}"));
        let managed = root.join("DemoGame_Data").join("Managed");
        fs::create_dir_all(&managed).unwrap();
        let dll = managed.join("Assembly-CSharp.dll");
        fs::write(&dll, build_synthetic_unity_dll_for_test()).unwrap();

        let discovered = discover_unity_assemblies(&root);
        assert_eq!(discovered, vec![dll.clone()]);

        let overview = list_types_in_assembly(
            &dll,
            vec!["Assembly-CSharp.dll".to_owned()],
            None,
            50,
            false,
        )
        .unwrap();
        assert_eq!(overview.total_types, 2);
        assert_eq!(overview.types[0].full_name, "Demo.BaseHealth");
        assert_eq!(overview.types[1].full_name, "Demo.PlayerController");

        let base_detail = inspect_type_in_assembly(&dll, "Demo.BaseHealth").unwrap();
        assert_eq!(base_detail.subclasses, vec!["Demo.PlayerController"]);

        let player_detail = inspect_type_in_assembly(&dll, "Demo.PlayerController").unwrap();
        assert_eq!(
            player_detail.inheritance_chain,
            vec![
                "Demo.PlayerController",
                "Demo.BaseHealth",
                "UnityEngine.MonoBehaviour"
            ]
        );
        assert!(player_detail.fields.iter().any(|f| f.name == "isImmortal"
            && f.field_type == "Boolean"
            && f.declaring_type == "Demo.BaseHealth"
            && f.bridge_accessible));
        assert!(
            player_detail
                .fields
                .iter()
                .any(|f| f.name == "staminaRecovery" && f.field_type == "Single")
        );
        assert!(
            player_detail
                .properties
                .iter()
                .any(|p| p.name == "AllAmmoInfinity"
                    && p.property_type == "Boolean"
                    && p.can_read
                    && p.can_write)
        );
        assert!(player_detail.methods.iter().any(|m| m.name == "ResetHealth"
            && m.return_type == "Void"
            && m.param_count == 0
            && m.invoke_method_compatible));

        let _ = fs::remove_dir_all(&root);
    }
}
