# CADCraft's native format (`.cad`) — records 1.3

> **Status:** draft, first version (branch `cad-native`) · **Lock file:** `.cdl` · **Code:**
> `crates/io/src/cad_file/` (format), `crates/engine/src/cmd/file/lock.rs` (lock)

`.cad` is CADCraft's own drawing file. It holds CADCraft's drawing model exactly as CADCraft
keeps it in memory, so a drawing saved and opened again is the same drawing, bit for bit, with
nothing translated through DXF or DWG.

It uses the container of WithShalom's open `cad-native` crate (MIT OR Apache-2.0): the same file
header, named sections, compression, CRCs, directory copies and recovery data. The records inside
are CADCraft's own. The file header says whose records a file holds (bytes 40–41: 0 the crate's
own, 1 CADCraft's), so each program tells the two apart by content and refuses the other's with a
message saying what it is.

This document has three parts: how CADCraft models a drawing (§1), the format (§2–§8), and
where the file differs from the model and why (§9). The lock file is §10.

---

## 1. CADCraft's drawing model

The model is the `cadcraft-doc` crate. Every type in it except the drawing itself, the entity
store and the kept foreign data derives serde's `Serialize` and `Deserialize`, with defaults on
almost every field; CADCraft already uses that form for its command parameters, the MCP and
control channel, and its own extension data inside DXF files. The format is built on it (§9.1).

### 1.1 The drawing

| Part | Type | What it is |
|---|---|---|
| `header` | `Header` (`vars: BTreeMap<String, HVal>`, `codes`) | Header variables by name without `$`: integers, reals, strings, 3D points; with the DXF group code each was read with |
| `layers`, `linetypes`, `text_styles`, `dim_styles`, `mleader_styles`, `table_styles`, `mline_styles` | `Vec<…>` in table order | Symbol tables and styles. Names are case-insensitive; the order is the order the UI lists |
| `blocks` | `BTreeMap<String, Arc<Block>>` | Block definitions: base point, flags, units, xref path, associative array parameters, and an entity store |
| `model` | `EntityStore` | Model space |
| `layouts` | `Vec<Layout>` | Paper-space layouts: tab order, page setup, saved paper view, and an entity store each |
| `views`, `vports`, `ucss` | | Named views, model-space viewport configurations (each tile with its view settings), named UCSs |
| `active_vport` | `Option<VportTile>` | The lone `*Active` viewport's handle, window and view settings (records 1.2) |
| `appids` | `Vec<AppId>` | Registered applications, in the case they were registered with (records 1.2) |
| `model_layout` | `Option<Layout>` | The Model layout's page setup and layout data (records 1.2) |
| `layer_states`, `groups`, `scales` | | LAYERSTATE, GROUP, the annotation scale list |
| `plot_style_tables` | | Plot style tables kept with the drawing (`.ctb`/`.stb` content) |
| `constraints`, `parametric` | | Geometric and dimensional constraints, user parameters and parametric settings |
| `handseed` | `u64` | The next free handle |
| `foreign` | `Arc<Foreign>` | What a DXF or DWG file held that CADCraft doesn't model: other programs' objects, extension dictionaries, reactors, xdata of table records, colour-book colours, the ACIS data store; kept as the file's own DXF groups and written back |
| `hidden_objects`, `read_notes`, `refedit`, `skipped_records` | | Session state: never saved |

An **entity store** keeps entities in draw order (chunks of `Arc<Entity>` with a handle
index), so draw order is the store's order; there is no separate sort table.

### 1.2 Entities

Every entity is `Entity { handle, common, kind }`.

`common` holds what every entity has: layer, colour (`ByLayer`, `ByBlock`, ACI index or true
colour), linetype, lineweight, linetype scale, transparency, visibility, thickness, extrusion
direction, named plot style, the annotation scales of an annotative object, and other
applications' xdata (raw DXF groups).

`kind` is one of 30 kinds. Geometry is two-dimensional in the XY plane with 3D points kept
where DXF keeps them:

| Kind | How CADCraft holds the geometry |
|---|---|
| Line | two 3D points |
| Point | 3D point and the angle of its PDMODE symbol |
| Circle, Arc | 3D centre, radius; arcs from `start` to `end` in radians, counter-clockwise |
| Ellipse | 3D centre, major-axis vector, minor/major ratio, start and end parameters |
| LwPolyline | 2D vertices with bulge (tan of a quarter of the arc angle) and start/end widths; closed, constant width, elevation, PLINEGEN |
| Polyline3d | 3D points, closed |
| Spline | a 2D (rational) B-spline: degree, knots, control points, weights, fit points, closed, and how it was fitted (knot parametrisation, end tangents, tolerance) so editing the fit points rebuilds it the same way |
| Helix | base centre, base and top radius, height, turns, start angle, twist, which value is constrained |
| Ray, XLine | base point and direction |
| Text, AttDef | insertion and alignment points, height, value, rotation, width factor, oblique, style, justification; attribute definitions add tag, prompt, modes, multiline text |
| MText | insertion point, height, wrap width, attachment, rotation, style, contents with inline codes, line spacing, columns |
| Insert | block name, insertion point, 3D scale, rotation, attributes, MINSERT columns and rows |
| Dimension | DXF's definition points (`defpt`, `p13`–`p16`, text middle), kind, text override, style, measurement, per-dimension style overrides, associativity (which definition point follows which object, and where on it), and its marks (oblique, jog, breaks, inspection) |
| MLine | style, the style's elements as used, scale, justification, vertices with cuts |
| Leader, MLeader | vertices, arrow, spline; multileaders hold leader lines, landing, dogleg, text or block content |
| Hatch | pattern name or solid, loops (closed bulge polylines, outer or island, and the objects each loop came from), scale, angle, associativity, style, gradient, origin, background, and the line families of a pattern not in the library |
| Solid, Trace, Face3d | four corners |
| Region | closed bulge loops (not an ACIS body) |
| Viewport | paper position and size, view centre and height, id, lock, frozen layers, per-viewport layer colours, clip entity, on/off |
| Image, Wipeout | image corner, pixel vectors, size, path, clip, display flags, brightness, contrast, fade; wipeouts as a 2D boundary |
| Table | columns, rows, cells (text, merges, cell style, formula, text height), row styles, breaks, data link |
| Tolerance | insertion point, direction, style, frame contents |
| Unknown | an entity type CADCraft doesn't model: its DXF type and groups, kept and written back |

### 1.3 Compared with `cad-native`'s own model

The `.cad` records follow CADCraft's model, not `cad-native`'s own (owner's decision, 2026-10-22). The
main differences, for anyone reading both formats: CADCraft's splines are 2D; its regions are
bulge loops rather than ACIS bodies (ACIS data from a file stays in the kept foreign data);
dimensions keep their definition points and regenerate their geometry; constraints, parametric
data, layer states and plot style tables are first-class parts of the drawing; and there are no
owner handles: an entity belongs to model space, a layout or a block by where it is stored.

---

## 2. The container

Everything in `cad-native`'s `docs/FORMAT.md` §1–§5 applies unchanged: little-endian values,
varints, the 48-byte header in two copies, the section directory in two copies, DEFLATE,
records split into streams before compression, CRC-32 per section, recovery parity between the
directory copies, the save to a temporary file renamed over the target. The header's format
version is the container's (2.10 today); the version of CADCraft's records is in `Meta` (§3.1).

What `.cad` sets differently:

- Header bytes 40–41 (records field) are **1**.
- Sections written: `Meta`, `Header`, `Schema`, `Objects.0` … `Objects.N`, `Preview`, and
  `Recovery`. Not written in 1.0: `Classes`, `ObjectMap`, `DwgPreserved`, `Digest`, `Journal.G`
  (§8).

## 3. Sections

### 3.1 `Meta`

A tag stream (no record framing): a `9` STR tag naming a variable, then its value tag. Unknown
names are skipped with their value.

| Name | Value |
|---|---|
| `$CADCRAFT_FORMAT` | `1` STR, the records version: `"1.3"` |
| `$CAD_WRITER` | `1` STR, e.g. `"CADCraft 0.10.0"` |
| `$CAD_OBJECTS` | `160` I64, records in the `Objects` sections (a check value) |
| `$CAD_PAGES` | `160` I64, number of `Objects.N` sections |

### 3.2 `Header`

A tag stream: for each header variable a `9` STR tag with its name (`$LTSCALE`), then its value
as a positional value (§5): integers in the smallest integer type, reals F64, strings STR,
points P3. The variables are written in name order, exactly as the drawing holds them.

From records 1.1, a variable read from a DXF or DWG file is followed by a `2001` I32 tag: the
DXF group code it was read with (−2 for a 2D point, −3 for a 3D point). Writers use it to write
back variables CADCraft doesn't know by name. Readers of 1.0 skip the tag.

The drawing's next free handle follows as `9 "$CADCRAFT_HANDSEED"` and a `5` HANDLE tag. (A
`$HANDSEED` variable the drawing carries from a DXF file is an ordinary variable here.)

### 3.3 `Schema`

A records section: one record per name in the file's **name dictionary**, in order, each a
single `2` STR tag. The *i*-th name (from 0) is written with codes 3000 + 2*i* and
3001 + 2*i* (§4). Names are struct field names, enum variant names and string map keys, as the
model's serde form spells them (`center`, `radius`, `byLayer`, `lwPolyline`…). A file has at
most 31 000 names; a map whose keys would not fit is written with positional keys (§5).

Because every file carries its own dictionary, a reader needs no table of codes, and a field
added to the model is saved by the next writer without a change to this document.

### 3.4 `Objects.0` … `Objects.N`

Records (§4), in the order of §6, packed into pages of about 64 KiB; a record is never split
across pages. Pages are read in number order.

### 3.5 `Preview`

An SVG of model space, as `cad-native` files have, so file browsers and Open dialogs can show the drawing.
At most 1 MiB before compression; left out when larger (`cad-native` allows 4 MiB). A hatched
drawing's SVG draws every hatch line and can be several megabytes, many times the drawing
itself; a lighter preview is later work.

### 3.6 `Recovery`

Parity over the snapshot's sections, as `cad-native` files have (FORMAT.md §7.7), so a damaged stretch of the
file can be rebuilt. CADCraft writes it; its reader doesn't apply it yet, because `cad-native`
keeps the repair private (a public `repair` in `cad-native` is a small `cad-native` change). Until
then a damaged section is read as it is, with a note (§7).

## 4. Records and codes

A record is one object: an entity, a table record, a block, a layout, or a piece of kept
foreign data. It is a varint byte length and then tags (FORMAT.md §6: `u16` code, `u8` type,
value). It starts with an **envelope**, then its **body**: the object's fields.

| Code | Type | Where | Meaning |
|---|---|---|---|
| 0 | STR | envelope, first tag | Record kind (§6), e.g. `LINE`, `LAYER`, `BLOCK` |
| 5 | HANDLE | envelope | The entity's handle (entities and their foreign extras) |
| 410 | STR | envelope | The layout whose paper space holds the entity |
| 2001 | STR | envelope | The block whose definition holds the entity |
| 0–1999 | STR | an item of a sequence | One raw DXF group, kept as read: its own DXF code and its value text |
| 2010 | scalar | positional | A scalar value without a name: a sequence item, a tuple element, a map key or value, a header variable |
| 2011 | I16 | positional | A compound value without a name; the value is its form (§5) |
| 2012 | BOOL | | End of the innermost sequence or map |
| 3000 + 2*i* | scalar | named | A scalar value of the field or key named by dictionary entry *i* |
| 3001 + 2*i* | I16 | named | A compound value of the field or key named by entry *i*; the value is its form (§5) |

Codes below 2000 keep their DXF meaning, as in every record of this container. Codes 2000 and up
are CADCraft's own in a CADCraft file (`cad-native`'s own codes mean other things in its own
files; a file holds one or the other).

The body of a record is a map: one named value per field, until the end of the record (no end
tag). Entities' bodies have the fields `common` and `kind`; the others are the type's own fields.

## 5. Values

Each value is written under a code: its name's code when it has a name (a struct field, a
string map key), or the positional codes. Scalars are one tag; compound values are an opening
tag whose I16 value says the form, then their contents, then (for sequences and maps) an end
tag `2012`.

| Model value | Written as |
|---|---|
| `bool` | BOOL |
| integer | I16 if it fits, else I32, else I64 |
| `u64` above `i64::MAX`, and every `Handle` | HANDLE (split streams store handles as differences, so lists of handles shrink) |
| `f32`, `f64` | F64, bit for bit (−0.0, infinities and NaN included) |
| `String`, `char` | STR |
| bytes | BYTES |
| `Vec2`, `Vec3` | P2, P3 |
| a raw DXF group (`RawTag`) in a sequence, code 0–1999 | one tag with that DXF code and the value text as STR |
| `None` | compound form 4 (no contents, no end tag) |
| unit, `()` | compound form 5 (no contents, no end tag) |
| `Some(v)` | `v` |
| sequence, tuple | form 1: the items, each positional, then `2012` |
| struct, map with string keys | form 2: each value under its name's code, then `2012` |
| map with other keys | form 3: key and value alternately, each positional, then `2012` |
| enum, unit variant | STR, the variant's name |
| enum, variant with data | form 2 with one entry: the value under the variant's name |
| newtype struct | its inner value |

Internally and adjacently tagged enums (an entity's `kind`, colours) are structs with their tag
field, as serde writes them. Readers take any integer type for any integer field, and an
integer where a real is expected.

## 6. Record kinds and order

Records are written in this order; readers accept any order (they load all records, then build
the drawing).

| Kind (code 0) | From | Body |
|---|---|---|
| `LTYPE`, `LAYER`, `STYLE`, `DIMSTYLE`, `MLEADERSTYLE`, `TABLESTYLE`, `MLINESTYLE` | the tables, in table order | the record's fields |
| `SCALE`, `UCS`, `VIEW`, `VPORT`, `LAYERSTATE`, `GROUP`, `PLOTSTYLETABLE`, `CONSTRAINT` | the lists, in order | the item's fields |
| `PARAMETRIC` | `parametric` | user parameters and parametric settings |
| `BLOCK` | each block, by name | the block's fields (not its entities) |
| entities of that block | in draw order | envelope `5` and `2001`, body `common` and `kind` |
| `LAYOUT` | each layout, in list order | the layout's fields: page setup with its plot settings, layout data (not its entities) |
| `ACTIVE_VPORT` (records 1.2) | the one `*Active` viewport a file held | its handle, window shape and view settings |
| `MODEL_LAYOUT` (records 1.2) | the Model layout | page setup and layout data, no entities |
| `APPID` (records 1.2) | each registered application | `name`, `handle`, `flags` |
| entities of that layout | in draw order | envelope `5` and `410` |
| entities of model space | in draw order | envelope `5` |
| `FOREIGN_CLASS`, `FOREIGN_OBJECT`, `FOREIGN_DICTIONARY`, `FOREIGN_EXTRA`, `FOREIGN_RECORD_EXTRA`, `FOREIGN_TARGET`, `FOREIGN_DATASTORE` | kept foreign data | its DXF groups as raw groups (§5), handles as HANDLE |
| `FOREIGN_CLASS_ORDER` (records 1.3) | the file's class names in its CLASSES order, when it has proxy objects or entities | `names`: a proxy's class number (group 91) is 500 plus its class's place here, and saving renumbers it to the written CLASSES |
| `FOREIGN_HANDLE` (records 1.1) | the handle a role had in the file read: a table, a record, a block's BLOCK or ENDBLK, the named object dictionary and its entries | `handle`, `table` (the role's kind), `name` |

An entity record's kind is its DXF type name (`LINE`, `LWPOLYLINE`, `ACAD_TABLE`…), for people
and tools reading the file; the body's `kind.type` field is what the reader uses.

## 7. Reading

- **Recognising.** A file is CADCraft's `.cad` when the container's magic is there and the
  records field is 1, whatever its name. A `cad-native` file with its own records (records field
  0) named `.cad` is refused with a message saying what it is.
- **Versions.** Records 1.x are read. A newer minor version is read with a note that fields a
  newer CADCraft added are not kept on save. A major version other than 1 is refused with a
  message naming it.
- **Damage.** A section that fails its CRC is read, with a note. A record that can't be parsed
  or doesn't fit the model is left out, counted in the drawing's skipped records for its
  section, and the drawing opens without it; the notes say how many. (Rebuilding damaged bytes
  from the recovery data first comes when `cad-native` makes its repair public, §3.6.)
- **Handles.** An entity whose handle another entity already has gets a new handle, with a note.
  The next free handle is raised above every handle read.
- **Limits.** Values nest at most 64 deep; the dictionary has at most 31 000 names; sections
  are bounded by the container's own checks. Nothing is allocated from a count in the file.
- **Not kept from a newer writer** (1.0): fields the reader's model doesn't have are dropped
  when it saves (§8).

## 8. Not in records 1.0

- **Incremental save** (`Journal.G` change sets) and the **SHA-256 hash chain** (`Digest`):
  CADCraft writes whole snapshots. The container supports both; they can be added without a new
  major version.
- **`ObjectMap`** (reading one object without the rest): CADCraft opens whole drawings.
- **Keeping unknown fields** from a newer CADCraft: serde drops fields a type doesn't have.
  Keeping them needs a record of which fields were read (planned for 1.1).
- **Save time and author** in `Meta`: left out so the same drawing always saves to the same
  bytes (and the web build has no clock); the lock file has who and when.

## 9. Notes: where the file differs from the model, and why

1. **Serde form instead of a hand-written layout per type** (optimal). The model already has a
   complete, defaulted serde form; writing it field by field under dictionary codes means every
   entity type, table and setting is saved, today and as the model grows, with one encoder and
   one decoder instead of one per type. `cad-native`'s own records name fields by fixed DXF group codes;
   CADCraft's fields have no DXF code (most of them aren't in DXF at all), so names are coded
   through the file's dictionary, which costs a few bytes per file and none per record.
2. **Points, handles and integers in their own tag types** (optimal). `Vec2`/`Vec3` as P2/P3
   (one tag instead of a map of two or three), handles as HANDLE (delta-coded by the split
   streams), integers in the smallest type that holds them.
3. **Raw DXF groups as their own tags** (optimal). Xdata, unknown entities and kept foreign
   objects are DXF groups already; each is one tag with its DXF code, so they stay readable as
   DXF and cost three bytes plus the value.
4. **Owners in the envelope instead of nesting** (necessary). Entity stores aren't serde types;
   each entity says where it lives (`410` layout, `2001` block, neither for model space), and
   record order is draw order. A damaged page then loses only its own entities, never moves an
   entity to the wrong block.
5. **Next free handle in `Header`** (necessary): it is a drawing field, not a header variable.
6. **`None` written out** (necessary): some optional fields are read through custom functions
   that don't treat a missing field as `None`.
7. **Exact numbers** (a gain over DXF): reals are stored bit for bit, where DXF writes decimal
   text. Settings DXF has no place for (for example a page setup's "Plot transparency") are kept.
8. **CADCraft's extras as records** (a gain over DXF): constraints, parametric data, layer states,
   plot style tables and annotation data are records of their own, not JSON inside DXF XRECORDs.
9. **Xrefs** (as DXF and DWG): an xref's block is saved without its objects, which come from the
   referenced file when the drawing opens; symbols that came with a loaded xref are not saved as
   the drawing's own; an xref's load status is read back as not loaded.
10. **Session state left out** (necessary): objects hidden by ISOLATEOBJECTS/HIDEOBJECTS, open
    notes, skipped-record counts, an ARRAYEDIT source being edited, and an in-place block edit
    in progress (the file is written from the drawing as REFCLOSE Discard would leave it).
11. **Kept foreign data under its own handles**: the handles of other programs' objects are
    the ones they had in the file they came from, as the DXF writer keeps them.

## 10. The lock file (`.cdl`)

While a `.cad` drawing is open for editing, CADCraft keeps `name.cdl` beside it (`part.cad` →
`part.cdl`): `cad-native`'s lock format under CADCraft's name (`cad_native::lock`,
`LockKind::CADCRAFT`; the first line is `cdl 1`), a few lines of text saying who has the drawing
open, on which machine, in which program, since when, and a heartbeat.

- **Opening** a `.cad` drawing takes its lock. If someone else holds it and their heartbeat is
  live, the drawing opens **read-only**: the open message says who has it, where and since when;
  saving to that file is refused, and Save As to another name works.
- **Heartbeat.** The desktop app refreshes the heartbeat every 30 seconds while the drawing is
  open. A lock not refreshed for 2 minutes is treated as left behind by a crash and taken over.
  Every save first checks that the lock is still this copy's (a script or MCP session may not
  keep the heartbeat): a file someone took over meanwhile isn't saved over, and a lock file that
  was only removed is taken again.
- **Saving as** a `.cad` file takes the new file's lock first; if someone else holds it, the save
  is refused. The old file's lock is released.
- **Closing** the drawing (or CADCraft) removes the lock file.
- Only `.cad` drawings are locked by CADCraft today. AutoCAD's own `.dwl` locks beside DWG files
  are separate work. The web build has no files and no locks.
- Like AutoCAD's `.dwl`, the lock is advisory: it coordinates copies of CADCraft (and WithShalom's other programs,
  which read `cdl` locks) on a shared folder; it does not stop other programs writing the file.

## Revision history

| Date | Change |
|---|---|
| 2026-10-28 | Records 1.3: `FOREIGN_CLASS_ORDER`, the class order a kept proxy is renumbered by (upstream #1812); without it a proxy in a drawing saved as `.cad` was dropped on the next DXF or DWG save |
| 2026-10-22 | Records 1.0: the model map, the format, notes, and the `.cdl` lock |
| 2026-10-23 | Records 1.2: plot settings and layout data, viewport settings, multileader and table style fields, dimension link snap data and flips travel in their records' fields; `APPID` and `MODEL_LAYOUT` records |
| 2026-10-23 | Records 1.1: header variables keep their DXF group codes (`2001` tag); tables, blocks, layouts and styles keep their handles (`handle` fields, `FOREIGN_HANDLE` records) |
