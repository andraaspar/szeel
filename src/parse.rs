//! Reads the KDL scene description.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::Arc;

use kdl::{KdlDocument, KdlEntry, KdlNode, KdlValue};
use miette::{Diagnostic, LabeledSpan, NamedSource, Severity, SourceSpan};

use crate::color::Oklch;
use crate::gradient::{Gradient, Stop};

/// Largest width or height accepted (the PNG limit).
pub const MAX_DIMENSION: u32 = i32::MAX as u32;

/// Most paths the image may draw, counting every `use` and mask in full.
pub const MAX_COST: u64 = 1_000_000;

/// Deepest nesting of groups, masks and `use`s, counted together.
pub const MAX_DEPTH: u32 = 64;

/// Index into [`Scene::paths`].
pub type PathId = u32;
/// Index into [`Scene::groups`].
pub type GroupId = u32;
/// Index into [`Scene::masks`].
pub type MaskId = u32;

/// A parsed file. Paths, groups and masks live in flat tables, in the order
/// they are completed in the file; the tree refers to them by index, so an
/// element used many times is stored once.
#[derive(Clone, Debug)]
pub struct Scene {
    pub width: u32,
    pub height: u32,
    pub paths: Vec<PathDef>,
    pub groups: Vec<GroupDef>,
    pub masks: Vec<Mask>,
    /// What is drawn, front first.
    pub root: Vec<Item>,
}

/// Something that can be drawn: a path or a group.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Target {
    Path(PathId),
    Group(GroupId),
}

/// One entry of a list of things to draw.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Item {
    Path(PathId),
    Group(GroupId),
    /// Draws an earlier definition again, optionally under a mask of its own.
    Use {
        target: Target,
        mask: Option<MaskId>,
    },
}

#[derive(Clone, Debug)]
pub struct PathDef {
    pub closed: bool,
    pub points: Vec<[f64; 2]>,
    /// Usable gradients, in file order.
    pub gradients: Vec<Gradient>,
    pub mask: Option<MaskId>,
    pub id: Option<String>,
    /// Location of the `path` node name.
    pub span: SourceSpan,
}

impl PathDef {
    /// The points without repeats (and, on a closed path, without a last
    /// point equal to the first), or `None` if fewer than 2 remain.
    pub fn distinct_points(&self) -> Option<Vec<[f64; 2]>> {
        let mut pts: Vec<[f64; 2]> = Vec::with_capacity(self.points.len());
        for &p in &self.points {
            if pts.last() != Some(&p) {
                pts.push(p);
            }
        }
        if self.closed && pts.len() > 1 && pts.first() == pts.last() {
            pts.pop();
        }
        (pts.len() >= 2).then_some(pts)
    }
}

#[derive(Clone, Debug)]
pub struct GroupDef {
    /// Children, front first.
    pub items: Vec<Item>,
    pub mask: Option<MaskId>,
    pub id: Option<String>,
    pub span: SourceSpan,
}

/// A mask: a group whose alpha scales the element it belongs to.
#[derive(Clone, Debug)]
pub struct Mask {
    pub invert: bool,
    pub items: Vec<Item>,
    /// A mask on the mask itself.
    pub mask: Option<MaskId>,
    pub span: SourceSpan,
}

/// An error or warning pointing at a place in the input.
#[derive(Debug, Clone)]
pub struct Diag {
    pub message: String,
    pub span: SourceSpan,
    pub label: Option<String>,
    /// Further places the message refers to.
    pub secondary: Vec<(SourceSpan, String)>,
    pub severity: Severity,
    pub help: Option<String>,
    pub source: Option<NamedSource<Arc<String>>>,
}

impl Diag {
    pub fn error(span: SourceSpan, message: impl Into<String>) -> Diag {
        Diag {
            message: message.into(),
            span,
            label: None,
            secondary: Vec::new(),
            severity: Severity::Error,
            help: None,
            source: None,
        }
    }

    pub fn warning(span: SourceSpan, message: impl Into<String>) -> Diag {
        Diag {
            severity: Severity::Warning,
            ..Diag::error(span, message)
        }
    }

    fn label(mut self, l: impl Into<String>) -> Diag {
        self.label = Some(l.into());
        self
    }

    fn also(mut self, span: SourceSpan, l: impl Into<String>) -> Diag {
        self.secondary.push((span, l.into()));
        self
    }

    fn help(mut self, h: impl Into<String>) -> Diag {
        self.help = Some(h.into());
        self
    }

    pub fn with_source(mut self, src: NamedSource<Arc<String>>) -> Diag {
        self.source = Some(src);
        self
    }
}

impl fmt::Display for Diag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for Diag {}

impl Diagnostic for Diag {
    fn severity(&self) -> Option<Severity> {
        Some(self.severity)
    }

    fn help<'a>(&'a self) -> Option<Box<dyn fmt::Display + 'a>> {
        self.help
            .as_ref()
            .map(|h| Box::new(h) as Box<dyn fmt::Display>)
    }

    fn source_code(&self) -> Option<&dyn miette::SourceCode> {
        self.source.as_ref().map(|s| s as &dyn miette::SourceCode)
    }

    fn labels(&self) -> Option<Box<dyn Iterator<Item = LabeledSpan> + '_>> {
        let primary = LabeledSpan::new_with_span(self.label.clone(), self.span);
        let rest = self
            .secondary
            .iter()
            .map(|(s, l)| LabeledSpan::new_with_span(Some(l.clone()), *s));
        Some(Box::new(std::iter::once(primary).chain(rest)))
    }
}

/// The result of parsing: the scene (if there were no errors) and all
/// diagnostics, errors and warnings, in file order.
pub struct Parsed {
    pub scene: Option<Scene>,
    pub diagnostics: Vec<Diag>,
}

/// Parses a scene. Syntax errors are returned as the `kdl` crate's own
/// diagnostic; everything else is reported through [`Parsed::diagnostics`].
pub fn parse(src: &str) -> Result<Parsed, kdl::KdlError> {
    let doc = KdlDocument::parse_v2(src)?;
    let mut p = Parser::default();
    let scene = p.document(&doc);
    let failed = p.diags.iter().any(|d| d.severity == Severity::Error);
    p.diags.sort_by_key(|d| d.span.offset());
    Ok(Parsed {
        scene: if failed { None } else { scene },
        diagnostics: p.diags,
    })
}

/// Expanded size of an element: how many paths drawing it takes, and how deep
/// the renderer has to nest to draw it.
#[derive(Clone, Copy, Default)]
struct Info {
    cost: u64,
    depth: u32,
}

/// Where a list of children is.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Ctx {
    Top,
    Group,
    Mask,
    Defs,
}

impl Ctx {
    fn allowed(self) -> &'static [&'static str] {
        match self {
            Ctx::Top => &["image", "path", "group", "use", "defs"],
            Ctx::Group | Ctx::Mask => &["path", "group", "use", "mask"],
            Ctx::Defs => &["path", "group"],
        }
    }
}

/// The `mask` child of an element, if any.
#[derive(Default)]
struct MaskSlot {
    first: Option<SourceSpan>,
    id: Option<MaskId>,
}

#[derive(Default)]
struct Parser {
    diags: Vec<Diag>,
    paths: Vec<PathDef>,
    groups: Vec<GroupDef>,
    masks: Vec<Mask>,
    path_info: Vec<Info>,
    group_info: Vec<Info>,
    mask_info: Vec<Info>,
    /// Every `id` in the file, at its first occurrence.
    all_ids: HashMap<String, SourceSpan>,
    /// The `id`s of completely read elements.
    ids: HashMap<String, Target>,
    /// The `id`s of elements that had errors; using them is not reported again.
    failed_ids: HashSet<String>,
    /// The `id`s of elements being read (the current element and its parents).
    open_ids: Vec<String>,
    used: HashSet<Target>,
    /// Elements directly inside `defs`.
    defs: Vec<(Target, SourceSpan, Option<String>)>,
    /// Current nesting of groups and masks.
    level: u32,
}

/// Marker for "an error was already recorded".
struct Failed;

type R<T> = Result<T, Failed>;

impl Parser {
    fn err(&mut self, d: Diag) -> Failed {
        self.diags.push(d);
        Failed
    }

    fn document(&mut self, doc: &KdlDocument) -> Option<Scene> {
        collect_ids(doc, &mut self.all_ids);
        let mut size: Option<(u32, u32)> = None;
        let mut first_element: Option<&str> = None;
        let mut root = Vec::new();
        let mut total = 0u64;
        let mut ok = true;
        for node in doc.nodes() {
            let name = node.name().value();
            if name == "image" {
                if size.is_some() {
                    self.err(
                        Diag::error(node.name().span(), "`image` given more than once")
                            .label("second `image`"),
                    );
                    ok = false;
                } else if let Some(prev) = first_element {
                    self.err(
                        Diag::error(node.name().span(), "`image` must come first")
                            .label(format!("after a `{prev}`")),
                    );
                    ok = false;
                }
                match self.image(node) {
                    Ok(s) => size = size.or(Some(s)),
                    Err(Failed) => ok = false,
                }
                continue;
            }
            if matches!(name, "path" | "group" | "use" | "defs") && first_element.is_none() {
                first_element = Some(name);
                if size.is_none() && ok {
                    self.err(
                        Diag::error(node.name().span(), format!("`{name}` before `image`"))
                            .help("start the file with `image width=… height=…`"),
                    );
                    ok = false;
                }
            }
            if name == "defs" {
                if self.defs(node).is_err() {
                    ok = false;
                }
                continue;
            }
            match self.child(node, Ctx::Top, None) {
                Ok(Some(item)) => {
                    let before = total;
                    total = total.saturating_add(self.item_info(item).cost);
                    if before <= MAX_COST && total > MAX_COST {
                        self.err(
                            Diag::error(
                                node.name().span(),
                                format!("the image draws more than {MAX_COST} paths in total"),
                            )
                            .label("the limit is crossed here")
                            .help("each `use` draws its element again, so uses of uses multiply"),
                        );
                        ok = false;
                    }
                    root.push(item);
                }
                Ok(None) => {}
                Err(Failed) => ok = false,
            }
        }
        if size.is_none() && ok {
            self.err(
                Diag::error(SourceSpan::from(0..0), "missing `image` node")
                    .help("start the file with `image width=… height=…`"),
            );
            return None;
        }
        for (target, span, id) in std::mem::take(&mut self.defs) {
            if !self.used.contains(&target) {
                let help = match id {
                    Some(id) => format!("draw it with `use \"{id}\"`, or remove it"),
                    None => "give it an `id` and draw it with `use`, or remove it".into(),
                };
                self.diags.push(
                    Diag::warning(span, "this definition is never used")
                        .label("never drawn")
                        .help(help),
                );
            }
        }
        let (width, height) = size?;
        ok.then(|| Scene {
            width,
            height,
            paths: std::mem::take(&mut self.paths),
            groups: std::mem::take(&mut self.groups),
            masks: std::mem::take(&mut self.masks),
            root,
        })
    }

    fn defs(&mut self, node: &KdlNode) -> R<()> {
        self.check_entries(node, 0, 0, &[])?;
        let mut ok = true;
        for child in node.children().map(|d| d.nodes()).unwrap_or_default() {
            match self.child(child, Ctx::Defs, None) {
                Ok(Some(item)) => {
                    let (target, id) = match item {
                        Item::Path(p) => (Target::Path(p), self.paths[p as usize].id.clone()),
                        Item::Group(g) => (Target::Group(g), self.groups[g as usize].id.clone()),
                        Item::Use { .. } => unreachable!("`use` is rejected in `defs`"),
                    };
                    self.defs.push((target, child.name().span(), id));
                }
                Ok(None) => {}
                Err(Failed) => ok = false,
            }
        }
        if ok { Ok(()) } else { Err(Failed) }
    }

    /// Reads one node of a list of children. Returns `None` for a `mask`,
    /// which is stored in `slot`.
    fn child(&mut self, node: &KdlNode, ctx: Ctx, slot: Option<&mut MaskSlot>) -> R<Option<Item>> {
        let span = node.name().span();
        match node.name().value() {
            "path" => self.path(node).map(|p| Some(Item::Path(p))),
            "group" => self.group(node).map(|g| Some(Item::Group(g))),
            "use" if ctx != Ctx::Defs => self.use_(node).map(Some),
            "use" => Err(self.err(
                Diag::error(span, "`use` is not allowed inside `defs`")
                    .label("inside `defs`")
                    .help("to define a copy, wrap it in a group with an `id`: `group id=\"…\" { use \"…\" }`"),
            )),
            "mask" => match slot {
                Some(s) => self.mask_child(node, s).map(|()| None),
                None => Err(self.err(
                    Diag::error(span, "`mask` must be inside a `path`, `group` or `use`")
                        .label("not inside an element"),
                )),
            },
            "image" => Err(self.err(Diag::error(span, "`image` must be at the top level").label("nested"))),
            "defs" => Err(self.err(Diag::error(span, "`defs` must be at the top level").label("nested"))),
            _ => Err(self.err(unknown_node(node, ctx.allowed()))),
        }
    }

    fn children(
        &mut self,
        doc: Option<&KdlDocument>,
        ctx: Ctx,
        mut slot: Option<&mut MaskSlot>,
    ) -> R<Vec<Item>> {
        let mut items = Vec::new();
        let mut ok = true;
        for node in doc.map(|d| d.nodes()).unwrap_or_default() {
            match self.child(node, ctx, slot.as_deref_mut()) {
                Ok(Some(i)) => items.push(i),
                Ok(None) => {}
                Err(Failed) => ok = false,
            }
        }
        if ok { Ok(items) } else { Err(Failed) }
    }

    fn mask_child(&mut self, node: &KdlNode, slot: &mut MaskSlot) -> R<()> {
        let span = node.name().span();
        if let Some(first) = slot.first {
            return Err(self.err(
                Diag::error(span, "an element can have only one `mask`")
                    .label("second `mask`")
                    .also(first, "first `mask` here")
                    .help("put all shapes in one `mask`, or put a `mask` inside the `mask` to intersect them"),
            ));
        }
        slot.first = Some(span);
        slot.id = Some(self.mask(node)?);
        Ok(())
    }

    fn image(&mut self, node: &KdlNode) -> R<(u32, u32)> {
        self.no_children(node)?;
        self.check_entries(node, 0, 0, &["width", "height"])?;
        let w = self.dimension(node, "width");
        let h = self.dimension(node, "height");
        Ok((w?, h?))
    }

    fn dimension(&mut self, node: &KdlNode, key: &str) -> R<u32> {
        let Some(e) = node.entry(key) else {
            return Err(self.err(
                Diag::error(node.name().span(), format!("missing `{key}`"))
                    .label(format!("needs `{key}=…`")),
            ));
        };
        match e.value() {
            KdlValue::Integer(i) if *i >= 1 && *i <= MAX_DIMENSION as i128 => Ok(*i as u32),
            _ => Err(self.err(
                Diag::error(
                    e.span(),
                    format!("`{key}` must be a whole number from 1 to {MAX_DIMENSION}"),
                )
                .label("here"),
            )),
        }
    }

    /// Reads the `id` property. Returns it (if valid) and whether it is free.
    fn id_prop(&mut self, node: &KdlNode) -> (Option<(String, SourceSpan)>, bool) {
        let Some(e) = node.entry("id") else {
            return (None, true);
        };
        let KdlValue::String(s) = e.value() else {
            self.err(Diag::error(e.span(), "`id` must be a string").label("not a string"));
            return (None, false);
        };
        if let Some(&first) = self.all_ids.get(s.as_str())
            && first.offset() != e.span().offset()
        {
            self.err(
                Diag::error(e.span(), format!("the `id` \"{s}\" is already taken"))
                    .label("duplicate")
                    .also(first, "first given here"),
            );
            return (Some((s.clone(), e.span())), false);
        }
        (Some((s.clone(), e.span())), true)
    }

    /// Reads an element that may have an `id`, making the `id` usable once
    /// the element is complete.
    fn element(
        &mut self,
        node: &KdlNode,
        body: impl FnOnce(&mut Self, Option<String>) -> R<u32>,
        target: fn(u32) -> Target,
    ) -> R<u32> {
        let (id, free) = self.id_prop(node);
        let name = id.as_ref().map(|(s, _)| s.clone());
        if let Some(n) = &name {
            self.open_ids.push(n.clone());
        }
        let r = body(self, name.clone());
        if name.is_some() {
            self.open_ids.pop();
        }
        match (r, free, name) {
            (Ok(i), true, name) => {
                if let Some(n) = name {
                    self.ids.insert(n, target(i));
                }
                Ok(i)
            }
            (_, _, name) => {
                if let Some(n) = name {
                    self.failed_ids.insert(n);
                }
                Err(Failed)
            }
        }
    }

    fn path(&mut self, node: &KdlNode) -> R<PathId> {
        self.check_entries(node, 0, 0, &["closed", "id"])?;
        self.element(node, |p, id| p.path_body(node, id), Target::Path)
    }

    fn path_body(&mut self, node: &KdlNode, id: Option<String>) -> R<PathId> {
        let span = node.name().span();
        let closed = match node.entry("closed") {
            None => false,
            Some(e) => match e.value() {
                KdlValue::Bool(b) => *b,
                _ => {
                    return Err(self.err(
                        Diag::error(e.span(), "`closed` must be `#true` or `#false`").label("here"),
                    ));
                }
            },
        };
        let mut points = Vec::new();
        let mut gradients = Vec::new();
        let mut slot = MaskSlot::default();
        let mut ok = true;
        for child in node.children().map(|d| d.nodes()).unwrap_or_default() {
            match child.name().value() {
                "point" => match self.point(child) {
                    Ok(p) => points.push(p),
                    Err(Failed) => ok = false,
                },
                "gradient" => match self.gradient(child, closed) {
                    Ok(Some(g)) => gradients.push(g),
                    Ok(None) => {}
                    Err(Failed) => ok = false,
                },
                "mask" => {
                    if self.mask_child(child, &mut slot).is_err() {
                        ok = false;
                    }
                }
                _ => {
                    self.err(unknown_node(child, &["point", "gradient", "mask"]));
                    ok = false;
                }
            }
        }
        if !ok {
            return Err(Failed);
        }
        if points.len() < 2 {
            return Err(self.err(
                Diag::error(span, "a path needs at least 2 points")
                    .label(format!("has {}", points.len())),
            ));
        }
        let def = PathDef {
            closed,
            points,
            gradients,
            mask: slot.id,
            id,
            span,
        };
        if def.gradients.is_empty() {
            self.diags.push(
                Diag::warning(span, "this path has no usable gradients and paints nothing")
                    .label("no gradients"),
            );
        } else if def.distinct_points().is_none() {
            self.diags.push(
                Diag::warning(
                    span,
                    "all points of this path are the same; it paints nothing",
                )
                .label("degenerate"),
            );
        }
        let mut info = Info { cost: 1, depth: 0 };
        if let Some(m) = slot.id {
            let mi = self.mask_info[m as usize];
            info = Info {
                cost: info.cost.saturating_add(mi.cost),
                depth: mi.depth,
            };
        }
        self.check_limits(info, span, "path")?;
        self.paths.push(def);
        self.path_info.push(info);
        Ok(self.paths.len() as u32 - 1)
    }

    fn group(&mut self, node: &KdlNode) -> R<GroupId> {
        self.check_entries(node, 0, 0, &["id"])?;
        self.element(node, |p, id| p.group_body(node, id), Target::Group)
    }

    fn group_body(&mut self, node: &KdlNode, id: Option<String>) -> R<GroupId> {
        let span = node.name().span();
        self.enter(span)?;
        let mut slot = MaskSlot::default();
        let items = self.children(node.children(), Ctx::Group, Some(&mut slot));
        self.level -= 1;
        let items = items?;
        if items.is_empty() {
            self.diags
                .push(Diag::warning(span, "this group is empty and paints nothing").label("empty"));
        }
        let info = self.list_info(&items, slot.id);
        self.check_limits(info, span, "group")?;
        self.groups.push(GroupDef {
            items,
            mask: slot.id,
            id,
            span,
        });
        self.group_info.push(info);
        Ok(self.groups.len() as u32 - 1)
    }

    fn mask(&mut self, node: &KdlNode) -> R<MaskId> {
        self.check_entries(node, 0, 0, &["invert"])?;
        let span = node.name().span();
        let invert = match node.entry("invert") {
            None => false,
            Some(e) => match e.value() {
                KdlValue::Bool(b) => *b,
                _ => {
                    return Err(self.err(
                        Diag::error(e.span(), "`invert` must be `#true` or `#false`").label("here"),
                    ));
                }
            },
        };
        self.enter(span)?;
        let mut slot = MaskSlot::default();
        let items = self.children(node.children(), Ctx::Mask, Some(&mut slot));
        self.level -= 1;
        let items = items?;
        if items.is_empty() {
            let msg = if invert {
                "this inverted mask is empty, so it has no effect"
            } else {
                "this mask is empty, so it hides its element completely"
            };
            self.diags.push(Diag::warning(span, msg).label("empty"));
        }
        let info = self.list_info(&items, slot.id);
        self.check_limits(info, span, "mask")?;
        self.masks.push(Mask {
            invert,
            items,
            mask: slot.id,
            span,
        });
        self.mask_info.push(info);
        Ok(self.masks.len() as u32 - 1)
    }

    fn use_(&mut self, node: &KdlNode) -> R<Item> {
        let span = node.name().span();
        let args = self.check_entries(node, 1, 1, &[])?;
        let e = args[0];
        let mut slot = MaskSlot::default();
        let mut ok = true;
        for child in node.children().map(|d| d.nodes()).unwrap_or_default() {
            let r = if child.name().value() == "mask" {
                self.mask_child(child, &mut slot)
            } else {
                Err(self.err(unknown_node(child, &["mask"])))
            };
            if r.is_err() {
                ok = false;
            }
        }
        let KdlValue::String(name) = e.value() else {
            return Err(self.err(
                Diag::error(e.span(), "`use` takes the `id` to draw, as a string")
                    .label("not a string"),
            ));
        };
        let target = self.lookup(name, e.span())?;
        if !ok {
            return Err(Failed);
        }
        self.used.insert(target);
        let item = Item::Use {
            target,
            mask: slot.id,
        };
        self.check_limits(self.item_info(item), span, "use")?;
        Ok(item)
    }

    /// Finds the element a `use` refers to, explaining why if it cannot.
    fn lookup(&mut self, name: &str, span: SourceSpan) -> R<Target> {
        if let Some(&t) = self.ids.get(name) {
            return Ok(t);
        }
        if self.open_ids.iter().any(|o| o == name) {
            let def = self.all_ids[name];
            return Err(self.err(
                Diag::error(span, format!("\"{name}\" cannot be used inside itself"))
                    .label("used inside its own definition")
                    .also(def, "defined here")
                    .help("an element can only use elements that are complete before it"),
            ));
        }
        if self.failed_ids.contains(name) {
            return Err(Failed);
        }
        if let Some(&def) = self.all_ids.get(name) {
            if def.offset() > span.offset() {
                return Err(self.err(
                    Diag::error(span, format!("\"{name}\" is defined later"))
                        .label("used before its definition")
                        .also(def, "defined here")
                        .help("an `id` can only be used after its element; move the definition up, for example into a `defs` block"),
                ));
            }
            // An earlier element with this `id` was not read because of other errors.
            return Err(Failed);
        }
        let mut close: Vec<(usize, &String)> = self
            .all_ids
            .keys()
            .map(|k| (levenshtein(name, k), k))
            .filter(|&(d, k)| d <= (k.chars().count().max(name.chars().count()) / 2).max(1))
            .collect();
        close.sort();
        let close: Vec<String> = close
            .iter()
            .take(3)
            .map(|(_, k)| format!("\"{k}\""))
            .collect();
        let help = match close.len() {
            0 => "give a `path` or `group` this `id` before the `use`".to_string(),
            1 => format!("did you mean {}?", close[0]),
            _ => format!("did you mean one of: {}?", close.join(", ")),
        };
        Err(self.err(
            Diag::error(span, format!("no element has the `id` \"{name}\""))
                .label("unknown `id`")
                .help(help),
        ))
    }

    fn enter(&mut self, span: SourceSpan) -> R<()> {
        if self.level >= MAX_DEPTH {
            return Err(self.err(
                Diag::error(span, format!("nested more than {MAX_DEPTH} levels deep"))
                    .label("too deep")
                    .help(format!(
                        "groups, masks and `use`s together can nest at most {MAX_DEPTH} levels"
                    )),
            ));
        }
        self.level += 1;
        Ok(())
    }

    fn check_limits(&mut self, info: Info, span: SourceSpan, what: &str) -> R<()> {
        if info.cost > MAX_COST {
            let label = if info.cost == u64::MAX {
                "too many paths".to_string()
            } else {
                format!("{} paths", info.cost)
            };
            return Err(self.err(
                Diag::error(
                    span,
                    format!("this `{what}` draws more than {MAX_COST} paths when expanded"),
                )
                .label(label)
                .help("each `use` draws its element again, so uses of uses multiply"),
            ));
        }
        if info.depth > MAX_DEPTH {
            return Err(self.err(
                Diag::error(
                    span,
                    format!("this `{what}` nests more than {MAX_DEPTH} levels deep when expanded"),
                )
                .label("too deep")
                .help(format!(
                    "groups, masks and `use`s together can nest at most {MAX_DEPTH} levels"
                )),
            ));
        }
        Ok(())
    }

    fn target_info(&self, t: Target) -> Info {
        match t {
            Target::Path(p) => self.path_info[p as usize],
            Target::Group(g) => self.group_info[g as usize],
        }
    }

    fn item_info(&self, item: Item) -> Info {
        match item {
            Item::Path(p) => self.path_info[p as usize],
            Item::Group(g) => self.group_info[g as usize],
            Item::Use { target, mask } => {
                let t = self.target_info(target);
                let m = mask.map(|m| self.mask_info[m as usize]).unwrap_or_default();
                Info {
                    cost: t.cost.saturating_add(m.cost),
                    depth: 1 + t.depth.max(m.depth),
                }
            }
        }
    }

    /// The size of a group or mask with these children and mask.
    fn list_info(&self, items: &[Item], mask: Option<MaskId>) -> Info {
        let mut r = mask.map(|m| self.mask_info[m as usize]).unwrap_or_default();
        for &i in items {
            let x = self.item_info(i);
            r = Info {
                cost: r.cost.saturating_add(x.cost),
                depth: r.depth.max(x.depth),
            };
        }
        Info {
            cost: r.cost,
            depth: r.depth + 1,
        }
    }

    fn point(&mut self, node: &KdlNode) -> R<[f64; 2]> {
        self.no_children(node)?;
        let args = self.check_entries(node, 2, 2, &[])?;
        Ok([self.number(args[0], "x")?, self.number(args[1], "y")?])
    }

    fn gradient(&mut self, node: &KdlNode, closed: bool) -> R<Option<Gradient>> {
        let args = self.check_entries(node, 1, 1, &[])?;
        let t = self.number(args[0], "t")?;
        let mut stops = Vec::new();
        let mut ok = true;
        if let Some(children) = node.children() {
            for child in children.nodes() {
                if child.name().value() != "stop" {
                    self.err(unknown_node(child, &["stop"]));
                    ok = false;
                    continue;
                }
                match self.stop(child) {
                    Ok(s) => {
                        if let Some(prev) = stops.last().map(|(s, _): &(Stop, SourceSpan)| s.dist)
                            && s.dist < prev
                        {
                            self.err(
                                Diag::error(
                                    child.entries()[0].span(),
                                    "stop distances must not decrease",
                                )
                                .label(format!("less than the previous stop's {prev}")),
                            );
                            ok = false;
                        }
                        stops.push((s, child.span()));
                    }
                    Err(Failed) => ok = false,
                }
            }
        }
        if !ok {
            return Err(Failed);
        }
        if stops.is_empty() {
            return Err(self.err(
                Diag::error(node.name().span(), "a gradient needs at least one stop")
                    .label("no stops"),
            ));
        }
        let span = args[0].span();
        if closed {
            if !((1.0..2.0).contains(&t) || (3.0..4.0).contains(&t)) {
                self.diags.push(
                    Diag::warning(
                        span,
                        "gradient ignored: on a closed path, `t` must be in [1, 2) or [3, 4)",
                    )
                    .label("outside both sides"),
                );
                return Ok(None);
            }
        } else if !(0.0..4.0).contains(&t) {
            return Err(self.err(
                Diag::error(span, "on an open path, `t` must be in [0, 4)").label("out of range"),
            ));
        }
        Ok(Some(Gradient {
            t: t as f32,
            stops: stops.into_iter().map(|(s, _)| s).collect(),
        }))
    }

    fn stop(&mut self, node: &KdlNode) -> R<Stop> {
        self.no_children(node)?;
        let args = self.check_entries(node, 4, 5, &[])?;
        let d = self.number(args[0], "distance")?;
        let l = self.number(args[1], "L")?;
        let c = self.number(args[2], "C")?;
        let h = self.number(args[3], "H")?;
        let a = match args.get(4) {
            Some(e) => self.number(e, "alpha")?,
            None => 1.0,
        };
        let range = |me: &mut Self, v: f64, ok: bool, e: &KdlEntry, msg: &str| -> R<()> {
            if ok {
                Ok(())
            } else {
                Err(me.err(Diag::error(e.span(), msg.to_string()).label(format!("is {v}"))))
            }
        };
        range(self, d, d >= 0.0, args[0], "`distance` must be 0 or more")?;
        range(
            self,
            l,
            (0.0..=1.0).contains(&l),
            args[1],
            "`L` must be from 0 to 1",
        )?;
        range(self, c, c >= 0.0, args[2], "`C` must be 0 or more")?;
        if let Some(e) = args.get(4) {
            range(
                self,
                a,
                (0.0..=1.0).contains(&a),
                e,
                "`alpha` must be from 0 to 1",
            )?;
        }
        Ok(Stop {
            dist: d as f32,
            color: Oklch::new(l as f32, c as f32, h as f32, a as f32),
        })
    }

    fn number(&mut self, e: &KdlEntry, what: &str) -> R<f64> {
        let v = match e.value() {
            KdlValue::Integer(i) => *i as f64,
            KdlValue::Float(f) => *f,
            _ => {
                return Err(self.err(
                    Diag::error(e.span(), format!("`{what}` must be a number"))
                        .label("not a number"),
                ));
            }
        };
        if !v.is_finite() {
            return Err(
                self.err(Diag::error(e.span(), format!("`{what}` must be finite")).label("here"))
            );
        }
        Ok(v)
    }

    fn no_children(&mut self, node: &KdlNode) -> R<()> {
        match node.children() {
            Some(c) => Err(self.err(
                Diag::error(
                    c.span(),
                    format!("`{}` does not take children", node.name().value()),
                )
                .label("unexpected block"),
            )),
            None => Ok(()),
        }
    }

    /// Checks argument count and property names; returns the arguments.
    fn check_entries<'n>(
        &mut self,
        node: &'n KdlNode,
        min: usize,
        max: usize,
        props: &[&str],
    ) -> R<Vec<&'n KdlEntry>> {
        let name = node.name().value();
        let mut args = Vec::new();
        let mut seen: Vec<&str> = Vec::new();
        let mut ok = true;
        for e in node.entries() {
            match e.name() {
                None => {
                    if args.len() == max {
                        self.err(
                            Diag::error(e.span(), format!("too many arguments for `{name}`"))
                                .label("unexpected"),
                        );
                        ok = false;
                    }
                    args.push(e);
                }
                Some(k) => {
                    let k = k.value();
                    if !props.contains(&k) {
                        let help = if props.is_empty() {
                            format!("`{name}` has no properties")
                        } else {
                            format!("`{name}` accepts: {}", props.join(", "))
                        };
                        self.err(
                            Diag::error(e.span(), format!("unknown property `{k}`"))
                                .label("unknown")
                                .help(help),
                        );
                        ok = false;
                    } else if seen.contains(&k) {
                        self.err(
                            Diag::error(e.span(), format!("`{k}` given more than once"))
                                .label("duplicate"),
                        );
                        ok = false;
                    }
                    seen.push(k);
                }
            }
        }
        if args.len() < min {
            let what = if min == max {
                format!("{min}")
            } else {
                format!("at least {min}")
            };
            self.err(
                Diag::error(
                    node.name().span(),
                    format!("`{name}` needs {what} argument(s)"),
                )
                .label(format!("has {}", args.len())),
            );
            ok = false;
        }
        if ok { Ok(args) } else { Err(Failed) }
    }
}

/// Collects the `id` of every `path` and `group` that the parser will read,
/// at its first occurrence.
fn collect_ids(doc: &KdlDocument, out: &mut HashMap<String, SourceSpan>) {
    for node in doc.nodes() {
        let name = node.name().value();
        if matches!(name, "path" | "group")
            && let Some(e) = node.entry("id")
            && let KdlValue::String(s) = e.value()
        {
            out.entry(s.clone()).or_insert(e.span());
        }
        if matches!(name, "path" | "group" | "mask" | "use" | "defs")
            && let Some(c) = node.children()
        {
            collect_ids(c, out);
        }
    }
}

/// Edit distance between two strings, in characters.
fn levenshtein(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut diag = row[0];
        row[0] = i + 1;
        for (j, &cb) in b.iter().enumerate() {
            let next = (row[j + 1] + 1)
                .min(row[j] + 1)
                .min(diag + usize::from(ca != cb));
            diag = row[j + 1];
            row[j + 1] = next;
        }
    }
    row[b.len()]
}

fn unknown_node(node: &KdlNode, allowed: &[&str]) -> Diag {
    Diag::error(
        node.name().span(),
        format!("unknown node `{}`", node.name().value()),
    )
    .label("unknown")
    .help(format!("expected one of: {}", allowed.join(", ")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn errors(src: &str) -> Vec<String> {
        let p = parse(src).expect("syntax");
        p.diagnostics
            .iter()
            .filter(|d| d.severity == Severity::Error)
            .map(|d| d.message.clone())
            .collect()
    }

    fn warnings(src: &str) -> Vec<String> {
        let p = parse(src).expect("syntax");
        assert!(p.scene.is_some(), "{:?}", p.diagnostics);
        p.diagnostics
            .iter()
            .filter(|d| d.severity == Severity::Warning)
            .map(|d| d.message.clone())
            .collect()
    }

    fn ok(src: &str) -> Scene {
        let p = parse(src).expect("syntax");
        assert!(p.diagnostics.is_empty(), "{:?}", p.diagnostics);
        p.scene.unwrap()
    }

    const IMG: &str = "image width=10 height=10\n";
    const P: &str = "path { point 0 0; point 1 1; gradient 0 { stop 0 0 0 0; } }";

    fn src(body: &str) -> String {
        format!("{IMG}{}", body.replace("$P", P))
    }

    #[test]
    fn plan_example_parses() {
        let src = r#"
image width=800 height=600
path {
    point 100 500
    point 400 150
    point 700 500
    gradient 1.5 {
        stop 0  0.75 0.18 40
        stop 60 0.75 0.18 40 0.0
    }
    gradient 3.5 {
        stop 0  0.55 0.12 250
        stop 80 0.55 0.12 250 0.0
    }
}
path closed=#true {
    point 200 200
    point 600 200
    point 400 450
    gradient 1 {
        stop 0 0.95 0.02 90
    }
}
"#;
        let s = ok(src);
        assert_eq!((s.width, s.height), (800, 600));
        assert_eq!(s.paths.len(), 2);
        assert_eq!(s.root, [Item::Path(0), Item::Path(1)]);
        assert!(!s.paths[0].closed && s.paths[1].closed);
        assert_eq!(s.paths[0].gradients[1].stops[1].color.a, 0.0);
        assert_eq!(s.paths[1].points[2], [400.0, 450.0]);
    }

    #[test]
    fn errors_are_reported() {
        assert_eq!(
            errors("path { }"),
            ["`path` before `image`", "a path needs at least 2 points"]
        );
        assert_eq!(
            errors("image width=0 height=5"),
            ["`width` must be a whole number from 1 to 2147483647"]
        );
        assert_eq!(
            errors("image width=5 height=5 depth=3"),
            ["unknown property `depth`"]
        );
        assert_eq!(
            errors("image width=5 height=5\nfoo"),
            ["unknown node `foo`"]
        );
        assert_eq!(
            errors(
                "image width=5 height=5\npath { point 0 0; point 1 1; gradient 4 { stop 0 0 0 0 } }"
            ),
            ["on an open path, `t` must be in [0, 4)"]
        );
        assert_eq!(
            errors(
                "image width=5 height=5\npath { point 0 0; point 1 1; gradient 1 { stop 2 0 0 0; stop 1 0 0 0 } }"
            ),
            ["stop distances must not decrease"]
        );
        assert_eq!(
            errors(
                "image width=5 height=5\npath { point 0 0; point 1 1; gradient 1 { stop 0 1.5 0 0 2 } }"
            ),
            ["`L` must be from 0 to 1"]
        );
        assert_eq!(
            errors("image width=5 height=5\npath { point 0; point 1 1 }"),
            ["`point` needs 2 argument(s)"]
        );
        assert_eq!(
            errors("image width=5 height=5\npath { point 0 0 1; point 1 1 }"),
            ["too many arguments for `point`"]
        );
        assert_eq!(
            errors("image width=5 height=5\npath { point 0 0; point 1 1; gradient 1 }"),
            ["a gradient needs at least one stop"]
        );
    }

    #[test]
    fn closed_gradients_outside_sides_warn() {
        let p = parse("image width=5 height=5\npath closed=#true { point 0 0; point 1 1; gradient 2.5 { stop 0 0 0 0 } }").unwrap();
        let msgs: Vec<_> = p
            .diagnostics
            .iter()
            .map(|d| (d.severity, d.message.as_str()))
            .collect();
        assert_eq!(msgs.len(), 2);
        assert!(msgs.iter().all(|(s, _)| *s == Severity::Warning));
        assert!(p.scene.unwrap().paths[0].gradients.is_empty());
    }

    #[test]
    fn degenerate_paths_warn_in_file_order() {
        let w = warnings(&src(
            "path { point 1 1; point 1 1; gradient 0 { stop 0 0 0 0; } }\npath { point 0 0; point 1 1 }\n\
             path closed=#true { point 2 2; point 2 2; point 2 2; gradient 1 { stop 0 0 0 0; } }",
        ));
        assert_eq!(
            w,
            [
                "all points of this path are the same; it paints nothing",
                "this path has no usable gradients and paints nothing",
                "all points of this path are the same; it paints nothing",
            ]
        );
    }

    #[test]
    fn groups() {
        let s = ok(&src("group { $P; group { $P; group { $P } }; $P }\n$P"));
        assert_eq!(s.paths.len(), 5);
        assert_eq!(s.groups.len(), 3);
        // Groups are stored as they are completed, innermost first.
        assert_eq!(s.groups[0].items, [Item::Path(2)]);
        assert_eq!(s.groups[1].items, [Item::Path(1), Item::Group(0)]);
        assert_eq!(
            s.groups[2].items,
            [Item::Path(0), Item::Group(1), Item::Path(3)]
        );
        assert_eq!(s.root, [Item::Group(2), Item::Path(4)]);
        assert_eq!(
            warnings(&src("group { }")),
            ["this group is empty and paints nothing"]
        );
        assert_eq!(
            errors(&src("group { image width=1 height=1 }")),
            ["`image` must be at the top level"]
        );
        assert_eq!(
            errors(&src("group { defs { } }")),
            ["`defs` must be at the top level"]
        );
        assert_eq!(
            errors(&src("group { point 1 1 }")),
            ["unknown node `point`"]
        );
        assert_eq!(
            errors(&format!("{P}\n{IMG}")),
            ["`path` before `image`", "`image` must come first"]
        );
    }

    #[test]
    fn masks() {
        let s = ok(&src(
            "group { mask { $P }; $P }\npath { mask invert=#true { $P; group { $P } }; point 0 0; point 1 1; gradient 0 { stop 0 0 0 0; } }",
        ));
        assert_eq!(s.masks.len(), 2);
        assert!(!s.masks[0].invert && s.masks[1].invert);
        assert_eq!(s.groups[0].mask, Some(0));
        assert_eq!(s.groups[0].items, [Item::Path(1)]);
        assert_eq!(s.paths[4].mask, Some(1));
        assert_eq!(s.masks[1].items, [Item::Path(2), Item::Group(1)]);
        // A mask on a mask.
        let s = ok(&src("group { mask { mask { $P }; $P }; $P }"));
        assert_eq!(s.masks[1].mask, Some(0));

        let p = parse(&src("group { mask { $P }; $P; mask { $P } }")).unwrap();
        let e: Vec<_> = p
            .diagnostics
            .iter()
            .filter(|d| d.severity == Severity::Error)
            .collect();
        assert_eq!(e.len(), 1);
        assert_eq!(e[0].message, "an element can have only one `mask`");
        assert_eq!(e[0].secondary.len(), 1);
        assert_eq!(e[0].labels().unwrap().count(), 2);
        assert_eq!(
            errors(&src("mask { $P }")),
            ["`mask` must be inside a `path`, `group` or `use`"]
        );
        assert_eq!(
            errors(&src("group { mask invert=1 { $P }; $P }")),
            ["`invert` must be `#true` or `#false`"]
        );
        assert_eq!(
            warnings(&src("group { mask { }; $P }")),
            ["this mask is empty, so it hides its element completely"]
        );
        assert_eq!(
            warnings(&src("group { mask invert=#true { }; $P }")),
            ["this inverted mask is empty, so it has no effect"]
        );
    }

    #[test]
    fn reuse() {
        let s = ok(&src(
            r#"defs { path id="a" { point 0 0; point 1 1; gradient 0 { stop 0 0 0 0; } } }
group id="g" { mask { use "a"; }; path id="b" { point 0 0; point 2 2; gradient 0 { stop 0 0 0 0; } } }
use "a"
use "g" { mask invert=#true { use "b"; } }
group { use "b"; mask { group id="deep" { use "a"; } } }
use "deep""#,
        ));
        assert_eq!(s.paths[0].id.as_deref(), Some("a"));
        assert_eq!(s.groups[0].id.as_deref(), Some("g"));
        assert_eq!(
            s.masks[0].items,
            [Item::Use {
                target: Target::Path(0),
                mask: None
            }]
        );
        assert_eq!(s.root[0], Item::Group(0));
        assert_eq!(
            s.root[1],
            Item::Use {
                target: Target::Path(0),
                mask: None
            }
        );
        assert_eq!(
            s.root[2],
            Item::Use {
                target: Target::Group(0),
                mask: Some(1)
            }
        );
        assert_eq!(
            s.masks[1].items,
            [Item::Use {
                target: Target::Path(1),
                mask: None
            }]
        );
        assert_eq!(
            s.root[4],
            Item::Use {
                target: Target::Group(1),
                mask: None
            }
        );
    }

    #[test]
    fn reuse_errors() {
        let first = |src: &str| {
            let p = parse(src).unwrap();
            p.diagnostics
                .into_iter()
                .find(|d| d.severity == Severity::Error)
                .unwrap()
        };
        let d = first(&src(r#"path id="a" { point 0 0; point 1 1 }
group id="a" { $P }"#));
        assert_eq!(d.message, "the `id` \"a\" is already taken");
        assert_eq!(d.secondary.len(), 1);
        assert!(d.secondary[0].0.offset() < d.span.offset());

        let d = first(&src(r#"defs { path id="circle" { point 0 0; point 1 1 } }
use "cirle""#));
        assert_eq!(d.message, "no element has the `id` \"cirle\"");
        assert_eq!(d.help.as_deref(), Some("did you mean \"circle\"?"));
        let d = first(&src(
            r#"defs { path id="ring" { point 0 0; point 1 1 }; path id="rang" { point 0 0; point 1 1 } }
use "rnig""#,
        ));
        assert_eq!(
            d.help.as_deref(),
            Some("did you mean one of: \"rang\", \"ring\"?")
        );
        let d = first(&src(r#"use "zzz""#));
        assert_eq!(
            d.help.as_deref(),
            Some("give a `path` or `group` this `id` before the `use`")
        );

        let d = first(&src(r#"use "later"
path id="later" { point 0 0; point 1 1 }"#));
        assert_eq!(d.message, "\"later\" is defined later");
        assert!(d.secondary[0].0.offset() > d.span.offset());

        assert_eq!(
            errors(&src(r#"group id="me" { use "me"; }"#)),
            ["\"me\" cannot be used inside itself"]
        );
        assert_eq!(
            errors(&src(
                r#"path id="me" { mask { use "me"; }; point 0 0; point 1 1 }"#
            )),
            ["\"me\" cannot be used inside itself"]
        );
        assert_eq!(
            errors(&src(
                r#"defs { path id="a" { point 0 0; point 1 1 }; use "a" }"#
            )),
            ["`use` is not allowed inside `defs`"]
        );
        assert_eq!(
            errors(&src(r#"group { defs { } }"#)),
            ["`defs` must be at the top level"]
        );
        assert_eq!(
            errors(&src(r#"defs { mask { } }"#)),
            ["`mask` must be inside a `path`, `group` or `use`"]
        );
        assert_eq!(
            errors(&src(r#"use 3"#)),
            ["`use` takes the `id` to draw, as a string"]
        );
        assert_eq!(
            errors(&src(r#"path id=3 { point 0 0; point 1 1 }"#)),
            ["`id` must be a string"]
        );
        // An element with errors is not reported again where it is used.
        assert_eq!(
            errors(&src(r#"path id="bad" { point 0 0 }
use "bad""#)),
            ["a path needs at least 2 points"]
        );
        // Unused definitions.
        assert_eq!(
            warnings(&src(
                r#"defs { path id="a" { point 0 0; point 1 1; gradient 0 { stop 0 0 0 0; } }; group { $P } }"#
            )),
            [
                "this definition is never used",
                "this definition is never used"
            ]
        );
    }

    /// `n` definitions, each using the previous one twice.
    fn doubling(n: usize) -> String {
        let mut s =
            src(r#"defs { path id="g0" { point 0 0; point 1 1; gradient 0 { stop 0 0 0 0; } }"#);
        for i in 1..=n {
            s += &format!(
                "\ngroup id=\"g{i}\" {{ use \"g{}\"; use \"g{}\" }}",
                i - 1,
                i - 1
            );
        }
        s += &format!(" }}\nuse \"g{n}\"\n");
        s
    }

    #[test]
    fn expansion_limits() {
        // 2^19 paths are fine; 2^20 cross the limit at "g20".
        assert!(parse(&doubling(19)).unwrap().scene.is_some());
        let s = doubling(60);
        let t = std::time::Instant::now();
        let p = parse(&s).unwrap();
        assert!(t.elapsed().as_secs_f64() < 1.0);
        let e: Vec<_> = p
            .diagnostics
            .iter()
            .filter(|d| d.severity == Severity::Error)
            .collect();
        assert_eq!(e.len(), 1, "{e:?}");
        assert_eq!(
            e[0].message,
            "this `group` draws more than 1000000 paths when expanded"
        );
        assert_eq!(e[0].span.offset(), s.find("group id=\"g20\"").unwrap());

        // Many top-level uses that only cross the limit together.
        let mut s = doubling(18);
        s += "use \"g18\"\nuse \"g18\"\nuse \"g18\"\n";
        assert_eq!(
            errors(&s),
            ["the image draws more than 1000000 paths in total"]
        );

        // Nesting, written out and through uses.
        // The `kdl` crate recurses deeply here; unoptimised, it needs more
        // than a test thread's default stack.
        let deep = src(&format!("{}{P}{}", "group { ".repeat(70), " }".repeat(70)));
        let e = std::thread::Builder::new()
            .stack_size(64 << 20)
            .spawn(move || errors(&deep))
            .unwrap()
            .join()
            .unwrap();
        assert_eq!(e, ["nested more than 64 levels deep"]);
        let mut s =
            src(r#"defs { path id="d0" { point 0 0; point 1 1; gradient 0 { stop 0 0 0 0; } }"#);
        for i in 1..40 {
            s += &format!("\ngroup id=\"d{i}\" {{ use \"d{}\" }}", i - 1);
        }
        s += " }\nuse \"d39\"\n";
        // Each level is a group and a `use`; the `use` inside "d33" crosses the limit.
        let e: Vec<_> = parse(&s)
            .unwrap()
            .diagnostics
            .into_iter()
            .filter(|d| d.severity == Severity::Error)
            .collect();
        assert_eq!(e.len(), 1);
        assert_eq!(
            e[0].message,
            "this `use` nests more than 64 levels deep when expanded"
        );
        assert!(
            e[0].span.offset() > s.find("group id=\"d33\"").unwrap()
                && e[0].span.offset() < s.find("group id=\"d34\"").unwrap()
        );
    }

    #[test]
    fn diagnostics_are_sorted() {
        let p = parse(&src(r#"defs { group { $P } }
path { point 0 0; point 1 1 }"#))
        .unwrap();
        let offs: Vec<_> = p.diagnostics.iter().map(|d| d.span.offset()).collect();
        assert_eq!(offs.len(), 2);
        assert!(offs[0] < offs[1]);
        assert_eq!(p.diagnostics[0].message, "this definition is never used");
    }
}
