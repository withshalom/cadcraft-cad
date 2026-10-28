#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;

use cadcraft_doc::{AnnoScale, Common, Entity, EntityKind, Layout, Lineweight, Space, Transparency};
use cadcraft_geom::{Vec2, Vec3};
use serde_json::json;

use super::*;

/// `v` through a record body's tags and back.
fn through_tags<T: Serialize + DeserializeOwned>(v: &T) -> T {
    let mut dict = Dict::default();
    let mut w = TagWriter::new();
    w.str(CODE_KIND, "TEST");
    let fields = match to_node(v).unwrap() {
        Node::Map(f) => f,
        other => vec![("value".into(), other)],
    };
    codec::put_body(&mut w, &fields, &mut dict).unwrap();
    let r = parse_record(&w.buf, &dict.names).unwrap();
    assert_eq!(r.kind, "TEST");
    obj(r.body).unwrap()
}

/// `d` with what reading adds (its notes) cleared, for comparing drawings.
fn plain(mut d: Drawing) -> Drawing {
    d.read_notes.clear();
    d
}

fn round_trip(d: &Drawing) -> Drawing {
    let bytes = write(d, None).unwrap();
    assert!(is_cad(&bytes));
    let back = read(&bytes).unwrap();
    assert!(back.read_notes.is_empty(), "{:?}", back.read_notes);
    assert!(back.skipped_records.is_empty(), "{:?}", back.skipped_records);
    back
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
enum Shape {
    Empty,
    Wrapped(f64),
    Pair(i32, String),
    Named { a: Vec2, b: Option<Vec3> },
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
struct Edge {
    neg_zero: f64,
    inf: f64,
    big: Handle,
    small: Handle,
    min: i64,
    max_u: u64,
    i16s: Vec<i16>,
    text: String,
    ch: char,
    unit: (),
    none: Option<String>,
    some: Option<String>,
    options: Vec<Option<f64>>,
    by_number: BTreeMap<u32, String>,
    by_name: BTreeMap<String, Vec<u8>>,
    shapes: Vec<Shape>,
    tuple: (bool, f32, Vec2),
    raw: Vec<RawTag>,
    raw_alone: RawTag,
    json: serde_json::Value,
}

#[test]
fn edge_values_survive_bit_for_bit() {
    let e = Edge {
        neg_zero: -0.0,
        inf: f64::NEG_INFINITY,
        big: Handle(u64::MAX),
        small: Handle(0x2A),
        min: i64::MIN,
        max_u: u64::MAX,
        i16s: vec![i16::MIN, -1, 0, i16::MAX],
        text: "Ünïcödé ✓ \u{0} \\P{\\fArial;x}".into(),
        ch: '∅',
        unit: (),
        none: None,
        some: Some(String::new()),
        options: vec![None, Some(1.5), None],
        by_number: BTreeMap::from([(1, "one".into()), (u32::MAX, "max".into())]),
        by_name: BTreeMap::from([("".into(), vec![]), ("bytes".into(), vec![0, 255])]),
        shapes: vec![Shape::Empty, Shape::Wrapped(2.0), Shape::Pair(-3, "x".into()), Shape::Named { a: Vec2::new(1.0, 2.0), b: None }],
        tuple: (true, 0.1, Vec2::new(-0.0, 7.0)),
        raw: vec![
            RawTag { code: 1001, value: "ACME".into() },
            RawTag { code: 1040, value: "1.5".into() },
            RawTag { code: 99_999, value: "out of range".into() },
        ],
        raw_alone: RawTag { code: 8, value: "named, not in a sequence".into() },
        json: json!({"a": [1, 2.5, null, true, "s"], "b": {"c": -7}}),
    };
    let back = through_tags(&e);
    assert_eq!(back, e);
    assert!(back.neg_zero.is_sign_negative());
    assert!(back.tuple.2.x.is_sign_negative());
    // NaN keeps its bits (it can't be compared with ==).
    let nan = f64::from_bits(0x7FF8_0000_0000_1234);
    assert_eq!(through_tags(&vec![nan])[0].to_bits(), nan.to_bits());
}

#[test]
fn points_handles_and_raw_groups_take_their_own_tags() {
    let mut dict = Dict::default();
    let mut w = TagWriter::new();
    let v = to_node(&(Vec3::new(1.0, 2.0, 3.0), Handle(7), vec![RawTag { code: 1000, value: "a".into() }])).unwrap();
    codec::put_body(&mut w, &[("v".into(), v)], &mut dict).unwrap();
    let tags = codec::read_tags(&w.buf).unwrap();
    let types: Vec<String> = tags.iter().map(|t| format!("{}:{}", t.code, t.value.type_name())).collect();
    assert_eq!(types, ["3001:int16", "2010:point3", "2010:handle", "2011:int16", "1000:string", "2012:bool", "2012:bool"]);
}

fn p(x: f64, y: f64) -> serde_json::Value {
    json!({"x": x, "y": y})
}
fn p3(x: f64, y: f64, z: f64) -> serde_json::Value {
    json!({"x": x, "y": y, "z": z})
}

/// One entity of every kind, built from the model's serde form so a kind added to the model
/// fails here until it is listed.
fn every_kind() -> Vec<EntityKind> {
    let text = json!({"insert": p3(1.0, 2.0, 0.0), "height": 2.5, "value": "Text", "halign": "center", "alignPt": p3(3.0, 2.0, 0.0)});
    let mtext =
        json!({"insert": p3(0.0, 0.0, 0.0), "height": 2.5, "contents": "A\\Pb", "columns": {"type": "dynamic", "count": 2, "heights": [1.0, 2.0]}});
    let loop_ = json!({"vertices": [{"p": p(0.0, 0.0)}, {"p": p(4.0, 0.0), "bulge": 0.5}, {"p": p(4.0, 3.0)}], "outer": true, "sources": [17, 18]});
    let kinds = vec![
        json!({"type": "line", "a": p3(0.0, 0.0, 0.0), "b": p3(10.0, -0.0, 1e-300)}),
        json!({"type": "point", "p": p3(1.0, 1.0, 0.0), "angle": 0.25}),
        json!({"type": "circle", "center": p3(5.0, 5.0, 0.0), "radius": 2.0}),
        json!({"type": "arc", "center": p3(0.0, 0.0, 0.0), "radius": 3.0, "start": 0.1, "end": 6.0}),
        json!({"type": "ellipse", "center": p3(0.0, 0.0, 0.0), "major": p3(4.0, 0.0, 0.0), "ratio": 0.5, "start": 0.0, "end": 6.2}),
        json!({"type": "lwPolyline", "vertices": [{"p": p(0.0, 0.0), "startWidth": 0.1}, {"p": p(1.0, 1.0)}], "closed": true, "plinegen": true}),
        json!({"type": "polyline3d", "points": [p3(0.0, 0.0, 0.0), p3(1.0, 2.0, 3.0)], "closed": false}),
        json!({"type": "spline", "degree": 3, "knots": [0, 0, 0, 0, 1, 1, 1, 1], "control": [p(0.0, 0.0), p(1.0, 2.0), p(3.0, 2.0), p(4.0, 0.0)], "weights": [1.0, 0.5, 0.5, 1.0], "fit": [p(0.0, 0.0)]}),
        json!({"type": "helix", "center": p3(0.0, 0.0, 0.0), "baseRadius": 1.0, "topRadius": 2.0, "height": 5.0, "turns": 3.0, "ccw": false}),
        json!({"type": "ray", "base": p3(0.0, 0.0, 0.0), "dir": p3(1.0, 0.0, 0.0)}),
        json!({"type": "xLine", "base": p3(0.0, 0.0, 0.0), "dir": p3(0.0, 1.0, 0.0)}),
        {
            let mut t = text.clone();
            t["type"] = json!("text");
            t
        },
        {
            let mut t = mtext.clone();
            t["type"] = json!("mText");
            t
        },
        json!({"type": "attDef", "tag": "PART", "text": text, "prompt": "Part number", "modes": {"lockPosition": true}, "multiline": {"mtext": mtext, "readAs": "A\nb"}}),
        json!({"type": "insert", "block": "BOLT", "insert": p3(2.0, 2.0, 0.0), "scale": p3(2.0, 2.0, 1.0), "attribs": [{"tag": "PART", "text": text, "props": {"color": {"kind": "index", "value": 3}}}], "cols": 2, "rows": 3, "colSpacing": 5.0}),
        json!({"type": "dimension", "kind": {"linear": {"rotation": 0.5}}, "defpt": p3(0.0, 5.0, 0.0), "textMid": p3(5.0, 5.0, 0.0), "p13": p3(0.0, 0.0, 0.0), "p14": p3(10.0, 0.0, 0.0),
               "overrides": {"arrowSize": 0.25, "textStyle": "ROMANS"}, "assoc": [{"point": "p13", "handle": 1, "snap": "start"}, {"point": "p14", "handle": 1, "snap": {"onCircle": {"angle": 1.0}}}],
               "marks": {"oblique": 0.3, "breaks": [{"at": p3(1.0, 0.0, 0.0), "size": 0.2, "auto": true}], "inspect": {"shape": 1, "label": "A", "rate": "100%"}}}),
        json!({"type": "dimension", "kind": {"ordinate": {"x_type": true}}, "defpt": p3(0.0, 0.0, 0.0), "textMid": p3(1.0, 1.0, 0.0)}),
        json!({"type": "dimension", "kind": "radius", "defpt": p3(0.0, 0.0, 0.0), "textMid": p3(1.0, 1.0, 0.0), "marks": {"jogged": {"center": p3(0.0, 0.0, 0.0), "jog": p3(1.0, 0.0, 0.0)}}}),
        json!({"type": "mLine", "style": "STANDARD", "vertices": [{"p": p3(0.0, 0.0, 0.0)}, {"p": p3(5.0, 0.0, 0.0)}], "scale": -1.0}),
        json!({"type": "leader", "vertices": [p3(0.0, 0.0, 0.0), p3(1.0, 1.0, 0.0)], "spline": true}),
        json!({"type": "mLeader", "leaders": [[p3(0.0, 0.0, 0.0)], []], "landing": p3(2.0, 2.0, 0.0), "dogleg": 0.36, "text": mtext}),
        json!({"type": "mLeader", "leaders": [], "landing": p3(2.0, 2.0, 0.0), "dogleg": 0.0, "text": null, "block": {"block": "BOLT", "insert": p3(2.0, 2.0, 0.0)}}),
        json!({"type": "hatch", "pattern": "ANSI31", "solid": false, "loops": [loop_], "angle": 0.785, "associative": true,
               "gradient": {"name": "LINEAR", "color1": {"kind": "index", "value": 5}, "color2": {"kind": "true", "value": [1, 2, 3]}},
               "patternLines": [{"angle": 0.0, "origin": [0.0, 0.0], "delta": [0.0, 0.125], "dashes": [0.25, -0.125]}]}),
        json!({"type": "solid", "corners": [p3(0.0, 0.0, 0.0), p3(1.0, 0.0, 0.0), p3(0.0, 1.0, 0.0), p3(1.0, 1.0, 0.0)]}),
        json!({"type": "trace", "corners": [p3(0.0, 0.0, 0.0), p3(1.0, 0.0, 0.0), p3(0.0, 1.0, 0.0), p3(1.0, 1.0, 0.0)]}),
        json!({"type": "face3d", "corners": [p3(0.0, 0.0, 0.0), p3(1.0, 0.0, 1.0), p3(0.0, 1.0, 2.0), p3(1.0, 1.0, 3.0)], "hidden_edges": 5}),
        json!({"type": "region", "loops": [loop_]}),
        json!({"type": "viewport", "center": p3(5.0, 4.0, 0.0), "width": 8.0, "height": 6.0, "viewCenter": p(0.0, 0.0), "viewHeight": 20.0, "id": 2, "frozenLayers": ["HIDDEN"],
               "layerColors": [["A", {"kind": "index", "value": 1}]], "clip": 99, "on": false}),
        json!({"type": "image", "insert": p3(0.0, 0.0, 0.0), "u": p3(0.01, 0.0, 0.0), "v": p3(0.0, 0.01, 0.0), "size": p(640.0, 480.0), "path": "photo.png", "clip": [p(0.0, 0.0), p(10.0, 10.0)], "clipping": true, "fade": 20}),
        json!({"type": "wipeout", "boundary": [p(0.0, 0.0), p(1.0, 0.0), p(1.0, 1.0)]}),
        json!({"type": "table", "insert": p3(0.0, 0.0, 0.0), "colWidths": [2.0, 3.0], "rowHeights": [0.5], "cells": [[{"text": "a", "merged": [1, 2]}, {"text": "=1+1", "formula": "=1+1", "textHeight": 0.2}]], "title": true, "rowStyles": ["Title"]}),
        json!({"type": "tolerance", "insert": p3(0.0, 0.0, 0.0), "text": "{\\Fgdt;j}%%v0.1"}),
        json!({"type": "unknown", "dxf_type": "ACME_WIDGET", "tags": [{"code": 10, "value": "1.0"}, {"code": 100, "value": "AcmeWidget"}]}),
    ];
    let (ok, bad): (Vec<_>, Vec<_>) =
        kinds.into_iter().map(|j| serde_json::from_value::<EntityKind>(j.clone()).map_err(|e| format!("{e}: {j}"))).partition(|r| r.is_ok());
    let bad: Vec<String> = bad.into_iter().filter_map(|r| r.err()).collect();
    assert!(bad.is_empty(), "{}", bad.join("\n"));
    ok.into_iter().filter_map(|r| r.ok()).collect()
}

fn rich_common(i: usize) -> Common {
    Common {
        layer: "PARTS".into(),
        color: cadcraft_doc::color::Color::Index(u8::try_from(i % 255 + 1).unwrap()),
        linetype: "HIDDEN".into(),
        lineweight: Lineweight::Mm100(35),
        ltscale: 0.5,
        transparency: Transparency::Percent(40),
        visible: !i.is_multiple_of(7),
        thickness: 0.25,
        extrusion: Vec3::new(0.0, 0.0, -1.0),
        plot_style: "Style 1".into(),
        anno_scales: if i.is_multiple_of(3) { vec![AnnoScale::unit()] } else { vec![] },
        xdata: if i.is_multiple_of(2) { vec![RawTag { code: 1001, value: "ACME".into() }, RawTag { code: 1070, value: "5".into() }] } else { vec![] },
    }
}

/// A drawing with every entity kind in model space, a layout and a block, and every table.
fn full_drawing() -> Drawing {
    let mut d = Drawing::new_metric();
    d.ensure_layer("PARTS");
    d.header.set("LTSCALE", HVal::Real(0.5));
    d.header.set("INSBASE", HVal::Point(Vec3::new(1.0, 2.0, 0.0)));
    d.header.set("PROJECTNAME", HVal::Str("MI-201".into()));
    d.header.set("ORTHOMODE", HVal::Int(1));
    let kinds = every_kind();
    let mut block = Block::new("BOLT");
    block.base = Vec3::new(1.0, 1.0, 0.0);
    block.description = "A bolt".into();
    d.layouts.push(Layout::new("Sheet A", 3));
    for (i, k) in kinds.iter().enumerate() {
        let common = rich_common(i);
        d.add(&Space::Model, common.clone(), k.clone()).unwrap();
        d.add(&Space::Paper("Sheet A".into()), common.clone(), k.clone()).unwrap();
        let h = d.new_handle();
        block.entities.push(Entity { handle: h, common, kind: k.clone() });
    }
    d.blocks.insert("BOLT".into(), Arc::new(block));
    d.groups.push(serde_json::from_value(json!({"name": "G", "description": "", "selectable": true, "members": [0x100, 0x101]})).unwrap());
    d.views.push(serde_json::from_value(json!({"name": "FRONT", "center": p(1.0, 2.0), "height": 10.0, "width": 20.0, "twist": 0.1})).unwrap());
    d.ucss.push(
        serde_json::from_value(json!({"name": "TILTED", "origin": p3(1.0, 0.0, 0.0), "xAxis": p3(0.0, 1.0, 0.0), "yAxis": p3(-1.0, 0.0, 0.0)}))
            .unwrap(),
    );
    d.layer_states.push(cadcraft_doc::LayerState { name: "ALL OFF".into(), layers: d.layers.clone() });
    d
}

/// The kind's place in [`every_kind`]'s list of kinds. The match has no catch-all, so a kind
/// added to the model doesn't compile until it is listed here and built there.
fn kind_index(k: &EntityKind) -> usize {
    match k {
        EntityKind::Line(_) => 0,
        EntityKind::Point(_) => 1,
        EntityKind::Circle(_) => 2,
        EntityKind::Arc(_) => 3,
        EntityKind::Ellipse(_) => 4,
        EntityKind::LwPolyline(_) => 5,
        EntityKind::Polyline3d(_) => 6,
        EntityKind::Spline(_) => 7,
        EntityKind::Helix(_) => 8,
        EntityKind::Ray(_) => 9,
        EntityKind::XLine(_) => 10,
        EntityKind::Text(_) => 11,
        EntityKind::MText(_) => 12,
        EntityKind::AttDef(_) => 13,
        EntityKind::Insert(_) => 14,
        EntityKind::Dimension(_) => 15,
        EntityKind::MLine(_) => 16,
        EntityKind::Leader(_) => 17,
        EntityKind::MLeader(_) => 18,
        EntityKind::Hatch(_) => 19,
        EntityKind::Solid(_) => 20,
        EntityKind::Trace(_) => 21,
        EntityKind::Face3d(_) => 22,
        EntityKind::Region(_) => 23,
        EntityKind::Viewport(_) => 24,
        EntityKind::Image(_) => 25,
        EntityKind::Wipeout(_) => 26,
        EntityKind::Table(_) => 27,
        EntityKind::Tolerance(_) => 28,
        EntityKind::Unknown(_) => 29,
    }
}

#[test]
fn the_test_drawing_has_every_entity_kind() {
    let seen: std::collections::BTreeSet<usize> = every_kind().iter().map(kind_index).collect();
    assert_eq!(seen, (0..30).collect());
}

#[test]
fn every_entity_kind_and_table_round_trips() {
    let d = full_drawing();
    let back = round_trip(&d);
    assert_eq!(back.model.len(), every_kind().len());
    for (a, b) in d.model.iter().zip(back.model.iter()) {
        assert_eq!(a, b, "{}", a.kind.type_name());
    }
    assert_eq!(plain(back), d);
}

#[test]
fn saving_twice_writes_the_same_bytes() {
    let d = full_drawing();
    let a = write(&d, Some("<svg/>".into())).unwrap();
    assert_eq!(a, write(&d, Some("<svg/>".into())).unwrap());
    assert_eq!(write(&read(&a).unwrap(), Some("<svg/>".into())).unwrap(), a);
}

#[test]
fn kept_foreign_data_round_trips() {
    let mut d = Drawing::default();
    let h = d.add(&Space::Model, Common::default(), every_kind().remove(0)).unwrap();
    let tags = |v: &[(i32, &str)]| v.iter().map(|(c, s)| RawTag { code: *c, value: (*s).into() }).collect::<Vec<_>>();
    let extra = ForeignExtra {
        xdict: Some(Handle(0x50)),
        reactors: vec![Handle(0x51)],
        xdata: tags(&[(1001, "ACME"), (1000, "x")]),
        color_book: Some(("RAL".into(), "1234".into())),
        props: tags(&[(347, "60")]),
    };
    let f = Foreign {
        class_order: vec!["ACME_WIDGET".into(), "ACDBPLACEHOLDER".into()],
        handles: Default::default(),
        classes: vec![tags(&[(1, "ACME_WIDGET"), (2, "AcmeWidget"), (90, "1025")])],
        objects: vec![ForeignObject { handle: Handle(0x50), kind: "XRECORD".into(), tags: tags(&[(100, "AcDbXrecord"), (280, "1"), (1, "kept")]) }],
        dictionary: vec![("ACME_DICT".into(), Handle(0x50))],
        entities: BTreeMap::from([(h, extra.clone())]),
        records: BTreeMap::from([(("LAYER".into(), "WALLS".into()), extra)]),
        targets: BTreeMap::from([(Handle(0x2), ("TABLE".into(), "LAYER".into()))]),
        data_store: tags(&[(0, "ACDSRECORD"), (320, "A1"), (310, "00FF")]),
    };
    d.foreign = Arc::new(f);
    assert_eq!(plain(round_trip(&d)), d);
}

#[test]
fn real_drawings_round_trip_exactly() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
    let mut n = 0;
    for e in std::fs::read_dir(&dir).unwrap().flatten() {
        let path = e.path();
        if path.extension().and_then(|x| x.to_str()) != Some("dxf") {
            continue;
        }
        let d = plain(crate::read(&std::fs::read(&path).unwrap(), "x.dxf").unwrap());
        assert_eq!(plain(round_trip(&d)), d, "{}", path.display());
        n += 1;
    }
    assert!(n > 0);
}

/// With `MI201_CORPUS` naming a folder of DWG and DXF files (CI: the MI-201 corpus), each
/// opens, saves as `.cad` and opens again unchanged.
#[test]
fn corpus_drawings_round_trip_exactly() {
    let Some(dir) = std::env::var_os("MI201_CORPUS") else { return };
    let mut n = 0;
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        let path = e.path();
        let name = path.to_string_lossy().to_string();
        if !matches!(path.extension().and_then(|x| x.to_str()).map(str::to_ascii_lowercase).as_deref(), Some("dwg" | "dxf")) {
            continue;
        }
        let Ok(d) = crate::read(&std::fs::read(&path).unwrap(), &name) else { continue };
        let d = plain(d);
        let back = read(&write(&d, None).unwrap()).unwrap();
        assert!(back.read_notes.is_empty(), "{name}: {:?}", back.read_notes);
        assert_eq!(plain(back), d, "{name}");
        n += 1;
    }
    assert!(n > 0, "no drawings in the corpus");
}

#[test]
fn cad_files_are_told_apart_by_content() {
    let mut d = Drawing::default();
    d.add(&Space::Model, Common::default(), every_kind().remove(2)).unwrap();
    let cad = crate::write(&d, "part.cad").unwrap();
    assert!(is_cad(&cad));
    // The content decides, whatever the name says.
    assert_eq!(plain(crate::read(&cad, "renamed.dxf").unwrap()), d);
    // A preview is kept for file browsers.
    let c = Container::parse(&cad).unwrap();
    assert!(c.entry(SEC_PREVIEW).is_some());
}

#[test]
fn session_state_and_xref_contents_are_not_saved() {
    let mut d = full_drawing();
    let first = d.model.iter().next().unwrap().handle;
    d.hidden_objects.insert(first);
    d.read_notes.push("from the last file".into());
    let mut x = Block::new("SITE");
    x.xref_path = Some("site.dwg".into());
    x.xref_status = XrefStatus::Loaded;
    x.entities.push(Entity::new(Handle(0x9000), every_kind().remove(0)));
    d.blocks.insert("SITE".into(), Arc::new(x));
    let back = round_trip(&d);
    assert!(back.hidden_objects.is_empty());
    let site = back.block("SITE").unwrap();
    assert!(site.entities.is_empty());
    assert_eq!(site.xref_status, XrefStatus::NotLoaded);
    assert_eq!(site.xref_path.as_deref(), Some("site.dwg"));
}

/// A file built from raw sections, for files no writer of this version makes.
fn crafted(format: &str, objects: Vec<Vec<u8>>, names: &[&str]) -> Vec<u8> {
    let mut meta = TagWriter::new();
    meta.str(9, "$CADCRAFT_FORMAT");
    meta.str(1, format);
    let mut schema = Vec::new();
    for n in names {
        let mut w = TagWriter::new();
        w.str(2, n);
        put_record(&mut schema, w);
    }
    let mut page = Vec::new();
    for o in objects {
        let mut w = TagWriter::new();
        w.encoded(&o);
        put_record(&mut page, w);
    }
    let mut b = Builder::new(Encoding::current(true, Level::Fast), false).for_records(RECORDS_CADCRAFT);
    b.section(SEC_META, meta.buf);
    b.records(SEC_SCHEMA, schema);
    b.records("Objects.0", page);
    b.finish(1, 0)
}

fn line_record(handle: u64, layout: Option<&str>) -> Vec<u8> {
    let mut dict = Dict::default();
    let mut w = TagWriter::new();
    w.str(CODE_KIND, "LINE");
    w.handle(CODE_HANDLE, TagHandle(handle));
    if let Some(l) = layout {
        w.str(CODE_LAYOUT, l);
    }
    let fields = vec![("common".into(), to_node(&Common::default()).unwrap()), ("kind".into(), to_node(&every_kind()[0]).unwrap())];
    codec::put_body(&mut w, &fields, &mut dict).unwrap();
    assert!(dict.names.len() < 40);
    w.buf
}

fn line_names() -> Vec<String> {
    let mut dict = Dict::default();
    let mut w = TagWriter::new();
    let fields = vec![("common".into(), to_node(&Common::default()).unwrap()), ("kind".into(), to_node(&every_kind()[0]).unwrap())];
    codec::put_body(&mut w, &fields, &mut dict).unwrap();
    dict.names
}

#[test]
fn a_newer_major_version_is_refused_and_a_newer_minor_noted() {
    let names = line_names();
    let names: Vec<&str> = names.iter().map(String::as_str).collect();
    let e = read(&crafted("2.0", vec![line_record(0x40, None)], &names)).unwrap_err().to_string();
    assert!(e.contains("version 2.0") && e.contains("1.x"), "{e}");
    let d = read(&crafted("1.7", vec![line_record(0x40, None)], &names)).unwrap();
    assert_eq!(d.model.len(), 1);
    assert!(d.read_notes.iter().any(|n| n.contains("newer CADCraft")), "{:?}", d.read_notes);
}

#[test]
fn damaged_and_unknown_records_are_left_out_with_notes() {
    let names = line_names();
    let names: Vec<&str> = names.iter().map(String::as_str).collect();
    let mut unknown = TagWriter::new();
    unknown.str(CODE_KIND, "FUTURE_THING");
    let mut no_kind = TagWriter::new();
    no_kind.f64(3000, 1.0);
    let mut bad_value = TagWriter::new();
    bad_value.str(CODE_KIND, "LAYER");
    bad_value.i16(3001, 99);
    let objects =
        vec![line_record(0x40, None), unknown.buf, no_kind.buf, bad_value.buf, line_record(0x41, Some("No such layout")), line_record(0x40, None)];
    let d = read(&crafted("1.0", objects, &names)).unwrap();
    assert_eq!(d.model.len(), 2, "both lines, the second with a new handle");
    assert!(d.model.iter().any(|e| e.handle == Handle(0x40)) && d.handseed > 0x41);
    let notes = d.read_notes.join("\n");
    assert!(notes.contains("3 record(s)") && notes.contains("FUTURE_THING"), "{notes}");
    assert!(notes.contains("new handles") && notes.contains("layout or block"), "{notes}");
    assert_eq!(d.skipped_records.values().sum::<usize>(), 4);
}

#[test]
fn damaged_files_never_panic() {
    let bytes = write(&full_drawing(), Some("<svg/>".into())).unwrap();
    let mut seed = 0x9E37_79B9_7F4A_7C15_u64;
    let mut next = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    for _ in 0..400 {
        let mut b = bytes.clone();
        for _ in 0..1 + next() % 8 {
            let i = usize::try_from(next() % b.len() as u64).unwrap();
            b[i] ^= u8::try_from(next() % 255 + 1).unwrap();
        }
        let _ = read(&b);
    }
    for n in [0, 8, 48, 100, bytes.len() / 2, bytes.len() - 49, bytes.len() - 1] {
        let _ = read(&bytes[..n]);
    }
    // Values nested past the limit are refused, not followed.
    let mut w = TagWriter::new();
    w.str(CODE_KIND, "LAYER");
    for _ in 0..100 {
        w.i16(3001, 1);
    }
    assert!(read(&crafted("1.0", vec![w.buf], &["name"])).unwrap().read_notes.iter().any(|n| n.contains("deeper")));
}
