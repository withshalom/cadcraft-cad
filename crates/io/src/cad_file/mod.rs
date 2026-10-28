//! CADCraft's native format, `.cad` (docs/cad-format.md): CADCraft's drawing model as records in
//! the container of WithShalom's open `cad-native` crate, marked as CADCraft's in the file header.
//!
//! The records hold the model's serde form ([`node`]) as tags under the codes of the file's name
//! dictionary ([`codec`]), so every entity type, table and setting is saved exactly, and a field
//! added to the model is saved without a change here.

mod codec;
mod node;

use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;

use cad_native::Level;
use cad_native::RECORDS_CADCRAFT;
use cad_native::cad_model::model::Handle as TagHandle;
use cad_native::container::{Builder, Container, Encoding};
use cad_native::tags::{TagWriter, Value, put_record, records};
use cadcraft_doc::foreign::{Foreign, ForeignExtra, ForeignObject};
use cadcraft_doc::{Block, Drawing, Entity, HVal, Handle, Header, RawTag, XrefStatus};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use self::codec::{CODE_BLOCK, CODE_HANDLE, CODE_KIND, CODE_LAYOUT, Dict, MAX_NAMES};
use self::node::{Node, from_node, to_node};
use crate::{IoError, Result};

/// The version of CADCraft's records this code writes (docs/cad-format.md §3.1).
pub const FORMAT: &str = "1.3";
/// In the `Header` section, after a variable's value: the DXF group code it was read with
/// (records 1.1).
const HEADER_CODE: u16 = 2001;
const MAJOR: u32 = 1;
const MINOR: u32 = 3;
/// Target size of one `Objects.N` page; a record is never split across pages.
const PAGE_TARGET: usize = 64 * 1024;
/// Previews larger than this are left out, so a preview never outweighs the drawing (a hatched
/// drawing's SVG, every hatch line a polyline, can be several megabytes).
const PREVIEW_LIMIT: usize = 1 << 20;
/// Notes about damaged records, at most.
const MAX_NOTES: usize = 20;

const SEC_META: &str = "Meta";
const SEC_HEADER: &str = "Header";
const SEC_SCHEMA: &str = "Schema";
const SEC_PREVIEW: &str = "Preview";
const OBJECTS: &str = "Objects.";
const HANDSEED: &str = "$CADCRAFT_HANDSEED";

/// Whether `bytes` are a CADCraft `.cad` file: the native container holding CADCraft's records,
/// whatever the file is named.
pub fn is_cad(bytes: &[u8]) -> bool {
    cad_native::looks_native(bytes) && cad_native::records_of(bytes) == Some(RECORDS_CADCRAFT)
}

fn err(e: impl std::fmt::Display) -> IoError {
    IoError::Format(format!("CADCraft file: {e}"))
}

// ---------------------------------------------------------------------------
// Kept foreign data as records (docs/cad-format.md §6)
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize)]
struct FTags {
    tags: Vec<RawTag>,
}

#[derive(Serialize, Deserialize)]
struct FObject {
    handle: Handle,
    kind: String,
    tags: Vec<RawTag>,
}

#[derive(Serialize, Deserialize)]
struct FDictEntry {
    key: String,
    handle: Handle,
}

#[derive(Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct FExtra {
    xdict: Option<Handle>,
    reactors: Vec<Handle>,
    xdata: Vec<RawTag>,
    color_book: Option<(String, String)>,
    props: Vec<RawTag>,
}

impl FExtra {
    fn of(x: &ForeignExtra) -> FExtra {
        FExtra { xdict: x.xdict, reactors: x.reactors.clone(), xdata: x.xdata.clone(), color_book: x.color_book.clone(), props: x.props.clone() }
    }
    fn into_extra(self) -> ForeignExtra {
        ForeignExtra { xdict: self.xdict, reactors: self.reactors, xdata: self.xdata, color_book: self.color_book, props: self.props }
    }
}

#[derive(Serialize, Deserialize)]
struct FRecordExtra {
    table: String,
    name: String,
    extra: FExtra,
}

/// The file's class names in its CLASSES order, by which a kept proxy is renumbered on save
/// (records 1.3).
#[derive(Serialize, Deserialize)]
struct FClassOrder {
    names: Vec<String>,
}

#[derive(Serialize, Deserialize)]
struct FTarget {
    handle: Handle,
    table: String,
    name: String,
}

// ---------------------------------------------------------------------------
// Writing
// ---------------------------------------------------------------------------

/// Where an entity lives.
enum Owner<'a> {
    Model,
    Layout(&'a str),
    Block(&'a str),
}

#[derive(Default)]
struct Out {
    dict: Dict,
    pages: Vec<Vec<u8>>,
    page: Vec<u8>,
    count: usize,
}

impl Out {
    fn push(&mut self, w: TagWriter) {
        put_record(&mut self.page, w);
        self.count += 1;
        if self.page.len() >= PAGE_TARGET {
            self.pages.push(std::mem::take(&mut self.page));
        }
    }

    /// A record of `kind` whose body is `v`'s fields (a value that isn't a struct is the field
    /// `value`).
    fn record<T: Serialize + ?Sized>(&mut self, kind: &str, handle: Option<Handle>, v: &T) -> Result<()> {
        let fields = match to_node(v).map_err(err)? {
            Node::Map(f) => f,
            other => vec![("value".to_string(), other)],
        };
        let mut w = TagWriter::new();
        w.str(CODE_KIND, kind);
        if let Some(h) = handle {
            w.handle(CODE_HANDLE, TagHandle(h.0));
        }
        codec::put_body(&mut w, &fields, &mut self.dict).map_err(err)?;
        self.push(w);
        Ok(())
    }

    fn entity(&mut self, e: &Entity, owner: &Owner) -> Result<()> {
        let mut w = TagWriter::new();
        w.str(CODE_KIND, e.kind.dxf_name());
        w.handle(CODE_HANDLE, TagHandle(e.handle.0));
        match owner {
            Owner::Model => {}
            Owner::Layout(n) => w.str(CODE_LAYOUT, n),
            Owner::Block(n) => w.str(CODE_BLOCK, n),
        }
        let fields = vec![("common".to_string(), to_node(&e.common).map_err(err)?), ("kind".to_string(), to_node(&e.kind).map_err(err)?)];
        codec::put_body(&mut w, &fields, &mut self.dict).map_err(err)?;
        self.push(w);
        Ok(())
    }

    fn list<T: Serialize>(&mut self, kind: &str, items: &[T]) -> Result<()> {
        items.iter().try_for_each(|t| self.record(kind, None, t))
    }
}

/// `d` as a `.cad` file, with `preview` (an SVG of model space) when given. The drawing is written
/// as a save leaves it: an in-place block edit discarded, xref blocks without their objects.
pub fn write(d: &Drawing, preview: Option<String>) -> Result<Vec<u8>> {
    let committed = d.committed();
    let view = crate::dxf_xref::saved_view(&committed);
    let d: &Drawing = view.as_ref().unwrap_or(&committed);
    let mut out = Out::default();

    out.list("LTYPE", &d.linetypes)?;
    out.list("LAYER", &d.layers)?;
    out.list("STYLE", &d.text_styles)?;
    out.list("DIMSTYLE", &d.dim_styles)?;
    out.list("MLEADERSTYLE", &d.mleader_styles)?;
    out.list("TABLESTYLE", &d.table_styles)?;
    out.list("MLINESTYLE", &d.mline_styles)?;
    out.list("SCALE", &d.scales)?;
    out.list("UCS", &d.ucss)?;
    out.list("VIEW", &d.views)?;
    out.list("VPORT", &d.vports)?;
    out.list("APPID", &d.appids)?;
    out.list("LAYERSTATE", &d.layer_states)?;
    out.list("GROUP", &d.groups)?;
    out.list("PLOTSTYLETABLE", &d.plot_style_tables)?;
    out.list("CONSTRAINT", &d.constraints)?;
    out.record("PARAMETRIC", None, &d.parametric)?;
    for b in d.blocks.values() {
        out.record("BLOCK", None, &**b)?;
        for e in b.entities.iter() {
            out.entity(e, &Owner::Block(&b.name))?;
        }
    }
    if let Some(v) = &d.active_vport {
        out.record("ACTIVE_VPORT", None, &**v)?;
    }
    if let Some(m) = &d.model_layout {
        out.record("MODEL_LAYOUT", None, &**m)?;
    }
    for l in &d.layouts {
        out.record("LAYOUT", None, l)?;
        for e in l.entities.iter() {
            out.entity(e, &Owner::Layout(&l.name))?;
        }
    }
    for e in d.model.iter() {
        out.entity(e, &Owner::Model)?;
    }
    write_foreign(&mut out, &d.foreign)?;
    if !out.page.is_empty() || out.pages.is_empty() {
        let last = std::mem::take(&mut out.page);
        out.pages.push(last);
    }

    let mut header = TagWriter::new();
    for (name, v) in &d.header.vars {
        header.str(9, &format!("${name}"));
        codec::put_positional(&mut header, &to_node(v).map_err(err)?, &mut out.dict).map_err(err)?;
        if let Some(code) = d.header.code(name) {
            header.i32(HEADER_CODE, code);
        }
    }
    header.str(9, HANDSEED);
    header.handle(5, TagHandle(d.handseed));

    let mut meta = TagWriter::new();
    meta.str(9, "$CADCRAFT_FORMAT");
    meta.str(1, FORMAT);
    meta.str(9, "$CAD_WRITER");
    meta.str(1, &format!("CADCraft {}", env!("CARGO_PKG_VERSION")));
    meta.str(9, "$CAD_OBJECTS");
    meta.i64(160, i64::try_from(out.count).unwrap_or(i64::MAX));
    meta.str(9, "$CAD_PAGES");
    meta.i64(160, i64::try_from(out.pages.len()).unwrap_or(i64::MAX));

    let mut schema = Vec::new();
    for n in &out.dict.names {
        let mut w = TagWriter::new();
        w.str(2, n);
        put_record(&mut schema, w);
    }

    let mut b = Builder::new(Encoding::current(true, Level::Default), true).for_records(RECORDS_CADCRAFT);
    b.section(SEC_META, meta.buf);
    b.section(SEC_HEADER, header.buf);
    b.records(SEC_SCHEMA, schema);
    for (i, p) in out.pages.into_iter().enumerate() {
        b.records(&format!("{OBJECTS}{i}"), p);
    }
    if let Some(svg) = preview.filter(|s| s.len() <= PREVIEW_LIMIT) {
        b.section(SEC_PREVIEW, svg.into_bytes());
    }
    Ok(b.finish(1, 0))
}

fn write_foreign(out: &mut Out, f: &Foreign) -> Result<()> {
    for c in &f.classes {
        out.record("FOREIGN_CLASS", None, &FTags { tags: c.clone() })?;
    }
    for o in &f.objects {
        out.record("FOREIGN_OBJECT", None, &FObject { handle: o.handle, kind: o.kind.clone(), tags: o.tags.clone() })?;
    }
    for (key, handle) in &f.dictionary {
        out.record("FOREIGN_DICTIONARY", None, &FDictEntry { key: key.clone(), handle: *handle })?;
    }
    for (h, x) in &f.entities {
        out.record("FOREIGN_EXTRA", Some(*h), &FExtra::of(x))?;
    }
    for ((table, name), x) in &f.records {
        out.record("FOREIGN_RECORD_EXTRA", None, &FRecordExtra { table: table.clone(), name: name.clone(), extra: FExtra::of(x) })?;
    }
    for (h, (table, name)) in &f.targets {
        out.record("FOREIGN_TARGET", None, &FTarget { handle: *h, table: table.clone(), name: name.clone() })?;
    }
    if !f.data_store.is_empty() {
        out.record("FOREIGN_DATASTORE", None, &FTags { tags: f.data_store.clone() })?;
    }
    for ((table, name), h) in &f.handles {
        out.record("FOREIGN_HANDLE", None, &FTarget { handle: *h, table: table.clone(), name: name.clone() })?;
    }
    if !f.class_order.is_empty() {
        out.record("FOREIGN_CLASS_ORDER", None, &FClassOrder { names: f.class_order.clone() })?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Reading
// ---------------------------------------------------------------------------

/// One record as read: its kind, envelope and body.
struct Rec {
    kind: String,
    handle: Option<u64>,
    layout: Option<String>,
    block: Option<String>,
    body: Vec<(String, Node)>,
}

fn parse_record(data: &[u8], names: &[String]) -> std::result::Result<Rec, String> {
    let tags = codec::read_tags(data)?;
    let kind = match tags.first() {
        Some(t) if t.code == CODE_KIND => match &t.value {
            Value::Str(s) => s.clone(),
            _ => return Err("the record kind isn't text".into()),
        },
        _ => return Err("a record without its kind".into()),
    };
    let mut r = Rec { kind, handle: None, layout: None, block: None, body: Vec::new() };
    let mut i = 1usize;
    // The envelope: every tag before the first named value. Codes a newer writer adds are skipped.
    while let Some(t) = tags.get(i).filter(|t| t.code < 3000) {
        match (t.code, &t.value) {
            (CODE_HANDLE, Value::Handle(h)) => r.handle = Some(*h),
            (CODE_LAYOUT, Value::Str(s)) => r.layout = Some(s.clone()),
            (CODE_BLOCK, Value::Str(s)) => r.block = Some(s.clone()),
            _ => {}
        }
        i += 1;
    }
    r.body = codec::body(&tags, &mut i, names)?;
    Ok(r)
}

/// A record body as `T`: the body itself, or its one field `value` for a value that isn't a
/// struct.
fn obj<T: DeserializeOwned>(body: Vec<(String, Node)>) -> std::result::Result<T, String> {
    let single = match body.as_slice() {
        [(k, v)] if k == "value" => Some(v.clone()),
        _ => None,
    };
    match from_node(Node::Map(body)) {
        Ok(t) => Ok(t),
        Err(e) => match single {
            Some(v) => from_node(v).map_err(|e| e.0),
            None => Err(e.0),
        },
    }
}

fn field(body: &mut Vec<(String, Node)>, name: &str) -> Option<Node> {
    let i = body.iter().position(|(k, _)| k == name)?;
    Some(body.swap_remove(i).1)
}

/// The drawing being read, before entities are placed.
#[derive(Default)]
struct Parts {
    d: Drawing,
    parametric: Option<cadcraft_doc::Parametric>,
    foreign: Foreign,
    entities: Vec<(Entity, Option<String>, Option<String>)>,
}

impl Parts {
    fn take(&mut self, r: Rec) -> std::result::Result<(), String> {
        let d = &mut self.d;
        if let Some(h) = r.handle
            && r.body.iter().any(|(k, _)| k == "common")
            && r.body.iter().any(|(k, _)| k == "kind")
        {
            let mut body = r.body;
            let common = field(&mut body, "common").map(from_node).transpose().map_err(|e| e.0)?.unwrap_or_default();
            let kind = from_node(field(&mut body, "kind").unwrap_or(Node::None)).map_err(|e| format!("{} {}: {}", r.kind, Handle(h).hex(), e.0))?;
            self.entities.push((Entity { handle: Handle(h), common, kind }, r.layout, r.block));
            return Ok(());
        }
        let b = r.body;
        match r.kind.as_str() {
            "LTYPE" => d.linetypes.push(obj(b)?),
            "LAYER" => d.layers.push(obj(b)?),
            "STYLE" => d.text_styles.push(obj(b)?),
            "DIMSTYLE" => d.dim_styles.push(obj(b)?),
            "MLEADERSTYLE" => d.mleader_styles.push(obj(b)?),
            "TABLESTYLE" => d.table_styles.push(obj(b)?),
            "MLINESTYLE" => d.mline_styles.push(obj(b)?),
            "SCALE" => d.scales.push(obj(b)?),
            "UCS" => d.ucss.push(obj(b)?),
            "VIEW" => d.views.push(obj(b)?),
            "VPORT" => d.vports.push(obj(b)?),
            "APPID" => d.appids.push(obj(b)?),
            "ACTIVE_VPORT" => d.active_vport = Some(Box::new(obj(b)?)),
            "MODEL_LAYOUT" => d.model_layout = Some(Box::new(obj(b)?)),
            "LAYERSTATE" => d.layer_states.push(obj(b)?),
            "GROUP" => d.groups.push(obj(b)?),
            "PLOTSTYLETABLE" => d.plot_style_tables.push(obj(b)?),
            "CONSTRAINT" => d.constraints.push(obj(b)?),
            "PARAMETRIC" => self.parametric = Some(obj(b)?),
            "BLOCK" => {
                let mut block: Block = obj(b)?;
                // An xref's objects come from its file when the drawing opens.
                if block.xref_path.is_some() {
                    block.xref_status = XrefStatus::default();
                }
                d.blocks.insert(block.name.clone(), Arc::new(block));
            }
            "LAYOUT" => d.layouts.push(obj(b)?),
            "FOREIGN_CLASS" => self.foreign.classes.push(obj::<FTags>(b)?.tags),
            "FOREIGN_OBJECT" => {
                let o: FObject = obj(b)?;
                self.foreign.objects.push(ForeignObject { handle: o.handle, kind: o.kind, tags: o.tags });
            }
            "FOREIGN_DICTIONARY" => {
                let e: FDictEntry = obj(b)?;
                self.foreign.dictionary.push((e.key, e.handle));
            }
            "FOREIGN_EXTRA" => {
                let h = r.handle.ok_or("FOREIGN_EXTRA without its entity's handle")?;
                self.foreign.entities.insert(Handle(h), obj::<FExtra>(b)?.into_extra());
            }
            "FOREIGN_RECORD_EXTRA" => {
                let x: FRecordExtra = obj(b)?;
                self.foreign.records.insert((x.table, x.name), x.extra.into_extra());
            }
            "FOREIGN_TARGET" => {
                let t: FTarget = obj(b)?;
                self.foreign.targets.insert(t.handle, (t.table, t.name));
            }
            "FOREIGN_DATASTORE" => self.foreign.data_store = obj::<FTags>(b)?.tags,
            "FOREIGN_HANDLE" => {
                let t: FTarget = obj(b)?;
                self.foreign.handles.insert((t.table, t.name), t.handle);
            }
            "FOREIGN_CLASS_ORDER" => self.foreign.class_order = obj::<FClassOrder>(b)?.names,
            other => return Err(format!("a record of kind {other} this version doesn't know, left out")),
        }
        Ok(())
    }
}

/// Read a `.cad` file. Damage is reported in the drawing's read notes and skipped records; the
/// drawing opens with what could be read.
pub fn read(bytes: &[u8]) -> Result<Drawing> {
    let c = Container::parse(bytes).map_err(err)?;
    if c.header.records != RECORDS_CADCRAFT {
        return Err(err("the file holds another program's records"));
    }
    let mut notes: Vec<String> = c.notes.clone();
    let section = |name: &str, notes: &mut Vec<String>| -> Option<Vec<u8>> {
        let e = c.entry(name)?;
        match c.read(e) {
            Ok((data, crc_ok)) => {
                if !crc_ok {
                    notes.push(format!("The file's {name} section is damaged (its checksum doesn't match); it was read as it is."));
                }
                Some(data)
            }
            Err(e) => {
                notes.push(format!("The file's {name} section could not be read: {e}"));
                None
            }
        }
    };

    // Meta: the records version.
    match section(SEC_META, &mut notes).map(|m| codec::read_tags(&m)) {
        Some(Ok(tags)) => {
            let version = tags.windows(2).find_map(|p| match (&p[0].value, &p[1].value) {
                (Value::Str(n), Value::Str(v)) if p[0].code == 9 && n == "$CADCRAFT_FORMAT" => Some(v.clone()),
                _ => None,
            });
            if let Some(v) = version {
                let mut parts = v.split('.').map(|p| p.trim().parse::<u32>().ok());
                let (major, minor) = (parts.next().flatten(), parts.next().flatten());
                if major != Some(MAJOR) {
                    return Err(err(format!(
                        "it holds CADCraft records version {v}; this CADCraft reads version {MAJOR}.x (save it from a newer CADCraft as DXF or DWG to open it here)"
                    )));
                }
                if minor.is_some_and(|m| m > MINOR) {
                    notes.push(format!(
                        "The file was saved by a newer CADCraft (records {v}); what that version added is not kept when the drawing is saved here."
                    ));
                }
            }
        }
        Some(Err(e)) => notes.push(format!("The file's Meta section could not be read: {e}")),
        None => notes.push("The file has no Meta section; it was read as records 1.0.".into()),
    }

    // Schema: the name dictionary.
    let mut names = Vec::new();
    if let Some(data) = section(SEC_SCHEMA, &mut notes) {
        match records(&data) {
            Ok(rs) => {
                for r in rs.into_iter().take(MAX_NAMES) {
                    let n = codec::read_tags(r).ok().and_then(|t| match t.into_iter().next() {
                        Some(t) if t.code == 2 => match t.value {
                            Value::Str(s) => Some(s),
                            _ => None,
                        },
                        _ => None,
                    });
                    names.push(n.unwrap_or_default());
                }
            }
            Err(e) => notes.push(format!("The file's name dictionary could not be read: {e}")),
        }
    }

    let mut parts = Parts::default();

    // Header variables and the next free handle.
    let mut handseed = 0x100u64;
    if let Some(data) = section(SEC_HEADER, &mut notes) {
        let mut vars = BTreeMap::new();
        let mut codes = BTreeMap::new();
        match codec::read_tags(&data) {
            Ok(tags) => {
                let mut i = 0usize;
                while let Some(t) = tags.get(i) {
                    i += 1;
                    let (9, Value::Str(name)) = (t.code, &t.value) else { continue };
                    if name == HANDSEED {
                        if let Some(Value::Handle(h)) = tags.get(i).map(|t| &t.value) {
                            handseed = *h;
                        }
                        i += 1;
                        continue;
                    }
                    match codec::value(&tags, &mut i, &names, 1) {
                        Ok(Some(n)) => match from_node::<HVal>(n) {
                            Ok(v) => {
                                let key = name.trim_start_matches('$').to_string();
                                // Records 1.1: the DXF group code it was read with.
                                if let Some(t) = tags.get(i).filter(|t| t.code == HEADER_CODE)
                                    && let Value::I32(c) = t.value
                                {
                                    codes.insert(key.clone(), c);
                                    i += 1;
                                }
                                vars.insert(key, v);
                            }
                            Err(e) => notes.push(format!("Header variable {name} left out: {e}")),
                        },
                        Ok(None) => {}
                        Err(e) => {
                            notes.push(format!("The file's header variables stop at {name}: {e}"));
                            break;
                        }
                    }
                }
            }
            Err(e) => notes.push(format!("The file's header variables could not be read: {e}")),
        }
        parts.d.header = Header { vars, codes };
    }

    // Start from empty tables: the file has every record the drawing had.
    {
        let d = &mut parts.d;
        d.layers.clear();
        d.linetypes.clear();
        d.text_styles.clear();
        d.dim_styles.clear();
        d.mleader_styles.clear();
        d.table_styles.clear();
        d.mline_styles.clear();
        d.layouts.clear();
        d.scales.clear();
    }

    // Objects, page by page.
    let mut pages: Vec<(u64, String)> =
        c.entries.iter().filter_map(|e| e.name.strip_prefix(OBJECTS).and_then(|n| n.parse::<u64>().ok()).map(|n| (n, e.name.clone()))).collect();
    pages.sort();
    pages.dedup_by(|a, b| a.0 == b.0);
    let mut skipped: BTreeMap<String, usize> = BTreeMap::new();
    let mut damage = Vec::new();
    for (_, page) in &pages {
        let Some(data) = section(page, &mut notes) else { continue };
        let rs = match records(&data) {
            Ok(rs) => rs,
            Err(e) => {
                notes.push(format!("The file's {page} section could not be split into records: {e}"));
                *skipped.entry(page.clone()).or_default() += 1;
                continue;
            }
        };
        for r in rs {
            if let Err(e) = parse_record(r, &names).and_then(|rec| parts.take(rec)) {
                *skipped.entry(page.clone()).or_default() += 1;
                if damage.len() < MAX_NOTES {
                    damage.push(e);
                }
            }
        }
    }
    let lost: usize = skipped.values().sum();
    if lost > 0 {
        notes.push(format!("{lost} record(s) of the file could not be read and were left out: {}", damage.join("; ")));
    }

    // The drawing: entities in their spaces and blocks, in draw order.
    let Parts { mut d, parametric, foreign, entities } = parts;
    d.parametric = parametric.unwrap_or_default();
    d.foreign = Arc::new(foreign);
    d.handseed = handseed;
    if let Some(max) = entities.iter().map(|(e, ..)| e.handle.0).max() {
        d.handseed = d.handseed.max(max.saturating_add(1));
    }
    let mut seen = HashSet::new();
    let (mut renumbered, mut homeless) = (0usize, 0usize);
    for (mut e, layout, block) in entities {
        if !seen.insert(e.handle) {
            e.handle = d.new_handle();
            renumbered += 1;
        }
        let placed = match (layout, block) {
            (Some(l), _) => d.layouts.iter_mut().find(|x| x.name == l).map(|x| x.entities.push(e)).is_some(),
            (None, Some(b)) => d.blocks.get_mut(&b).map(|x| Arc::make_mut(x).entities.push(e)).is_some(),
            (None, None) => {
                d.model.push(e);
                true
            }
        };
        if !placed {
            homeless += 1;
        }
    }
    if renumbered > 0 {
        notes.push(format!("{renumbered} object(s) had the handle of another object and were given new handles."));
    }
    if homeless > 0 {
        notes.push(format!("{homeless} object(s) belonged to a layout or block the file doesn't have and were left out."));
        *skipped.entry("Objects".into()).or_default() += homeless;
    }
    d.read_notes = notes;
    d.skipped_records = skipped;
    Ok(d)
}

#[cfg(test)]
mod tests;
