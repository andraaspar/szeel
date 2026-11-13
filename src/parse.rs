//! Reads the KDL scene description.

use std::fmt;
use std::sync::Arc;

use kdl::{KdlDocument, KdlEntry, KdlNode, KdlValue};
use miette::{Diagnostic, LabeledSpan, NamedSource, Severity, SourceSpan};

use crate::color::Oklch;
use crate::gradient::{Gradient, Stop};

/// Largest width or height accepted (the PNG limit).
pub const MAX_DIMENSION: u32 = i32::MAX as u32;

#[derive(Clone, Debug)]
pub struct Scene {
    pub width: u32,
    pub height: u32,
    pub paths: Vec<PathDef>,
}

#[derive(Clone, Debug)]
pub struct PathDef {
    pub closed: bool,
    pub points: Vec<[f64; 2]>,
    /// Usable gradients, in file order.
    pub gradients: Vec<Gradient>,
    /// Location of the `path` node name, for later warnings.
    pub span: SourceSpan,
}

/// An error or warning pointing at a place in the input.
#[derive(Debug, Clone)]
pub struct Diag {
    pub message: String,
    pub span: SourceSpan,
    pub label: Option<String>,
    pub severity: Severity,
    pub help: Option<String>,
    pub source: Option<NamedSource<Arc<String>>>,
}

impl Diag {
    fn error(span: SourceSpan, message: impl Into<String>) -> Diag {
        Diag { message: message.into(), span, label: None, severity: Severity::Error, help: None, source: None }
    }

    pub fn warning(span: SourceSpan, message: impl Into<String>) -> Diag {
        Diag { severity: Severity::Warning, ..Diag::error(span, message) }
    }

    fn label(mut self, l: impl Into<String>) -> Diag {
        self.label = Some(l.into());
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
        self.help.as_ref().map(|h| Box::new(h) as Box<dyn fmt::Display>)
    }

    fn source_code(&self) -> Option<&dyn miette::SourceCode> {
        self.source.as_ref().map(|s| s as &dyn miette::SourceCode)
    }

    fn labels(&self) -> Option<Box<dyn Iterator<Item = LabeledSpan> + '_>> {
        Some(Box::new(std::iter::once(LabeledSpan::new_with_span(self.label.clone(), self.span))))
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
    let mut p = Parser { diags: Vec::new() };
    let scene = p.document(&doc);
    let failed = p.diags.iter().any(|d| d.severity == Severity::Error);
    Ok(Parsed { scene: if failed { None } else { scene }, diagnostics: p.diags })
}

struct Parser {
    diags: Vec<Diag>,
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
        let mut size: Option<(u32, u32)> = None;
        let mut paths = Vec::new();
        let mut ok = true;
        for node in doc.nodes() {
            let name = node.name().value();
            match name {
                "image" => {
                    if size.is_some() {
                        self.err(Diag::error(node.name().span(), "`image` given more than once").label("second `image`"));
                        ok = false;
                    } else if !paths.is_empty() {
                        self.err(
                            Diag::error(node.name().span(), "`image` must come before any `path`")
                                .label("after a `path`"),
                        );
                        ok = false;
                    }
                    match self.image(node) {
                        Ok(s) => size = size.or(Some(s)),
                        Err(Failed) => ok = false,
                    }
                }
                "path" => {
                    if size.is_none() && ok {
                        self.err(
                            Diag::error(node.name().span(), "`path` before `image`")
                                .help("start the file with `image width=… height=…`"),
                        );
                        ok = false;
                    }
                    match self.path(node) {
                        Ok(p) => paths.push(p),
                        Err(Failed) => ok = false,
                    }
                }
                _ => {
                    self.err(unknown_node(node, &["image", "path"]));
                    ok = false;
                }
            }
        }
        if size.is_none() && ok {
            self.err(
                Diag::error(SourceSpan::from(0..0), "missing `image` node")
                    .help("start the file with `image width=… height=…`"),
            );
            return None;
        }
        let (width, height) = size?;
        ok.then_some(Scene { width, height, paths })
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
            return Err(self.err(Diag::error(node.name().span(), format!("missing `{key}`")).label(format!("needs `{key}=…`"))));
        };
        match e.value() {
            KdlValue::Integer(i) if *i >= 1 && *i <= MAX_DIMENSION as i128 => Ok(*i as u32),
            _ => Err(self.err(
                Diag::error(e.span(), format!("`{key}` must be a whole number from 1 to {MAX_DIMENSION}")).label("here"),
            )),
        }
    }

    fn path(&mut self, node: &KdlNode) -> R<PathDef> {
        self.check_entries(node, 0, 0, &["closed"])?;
        let closed = match node.entry("closed") {
            None => false,
            Some(e) => match e.value() {
                KdlValue::Bool(b) => *b,
                _ => return Err(self.err(Diag::error(e.span(), "`closed` must be `#true` or `#false`").label("here"))),
            },
        };
        let mut points = Vec::new();
        let mut gradients = Vec::new();
        let mut ok = true;
        if let Some(children) = node.children() {
            for child in children.nodes() {
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
                    _ => {
                        self.err(unknown_node(child, &["point", "gradient"]));
                        ok = false;
                    }
                }
            }
        }
        if !ok {
            return Err(Failed);
        }
        if points.len() < 2 {
            return Err(self.err(
                Diag::error(node.name().span(), "a path needs at least 2 points")
                    .label(format!("has {}", points.len())),
            ));
        }
        if gradients.is_empty() {
            self.diags.push(
                Diag::warning(node.name().span(), "this path has no usable gradients and paints nothing")
                    .label("no gradients"),
            );
        }
        Ok(PathDef { closed, points, gradients, span: node.name().span() })
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
                                Diag::error(child.entries()[0].span(), "stop distances must not decrease")
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
            return Err(self.err(Diag::error(node.name().span(), "a gradient needs at least one stop").label("no stops")));
        }
        let span = args[0].span();
        if closed {
            if !((1.0..2.0).contains(&t) || (3.0..4.0).contains(&t)) {
                self.diags.push(
                    Diag::warning(span, "gradient ignored: on a closed path, `t` must be in [1, 2) or [3, 4)")
                        .label("outside both sides"),
                );
                return Ok(None);
            }
        } else if !(0.0..4.0).contains(&t) {
            return Err(self.err(Diag::error(span, "on an open path, `t` must be in [0, 4)").label("out of range")));
        }
        Ok(Some(Gradient { t: t as f32, stops: stops.into_iter().map(|(s, _)| s).collect() }))
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
            if ok { Ok(()) } else { Err(me.err(Diag::error(e.span(), msg.to_string()).label(format!("is {v}")))) }
        };
        range(self, d, d >= 0.0, args[0], "`distance` must be 0 or more")?;
        range(self, l, (0.0..=1.0).contains(&l), args[1], "`L` must be from 0 to 1")?;
        range(self, c, c >= 0.0, args[2], "`C` must be 0 or more")?;
        if let Some(e) = args.get(4) {
            range(self, a, (0.0..=1.0).contains(&a), e, "`alpha` must be from 0 to 1")?;
        }
        Ok(Stop { dist: d as f32, color: Oklch::new(l as f32, c as f32, h as f32, a as f32) })
    }

    fn number(&mut self, e: &KdlEntry, what: &str) -> R<f64> {
        let v = match e.value() {
            KdlValue::Integer(i) => *i as f64,
            KdlValue::Float(f) => *f,
            _ => return Err(self.err(Diag::error(e.span(), format!("`{what}` must be a number")).label("not a number"))),
        };
        if !v.is_finite() {
            return Err(self.err(Diag::error(e.span(), format!("`{what}` must be finite")).label("here")));
        }
        Ok(v)
    }

    fn no_children(&mut self, node: &KdlNode) -> R<()> {
        match node.children() {
            Some(c) => Err(self.err(
                Diag::error(c.span(), format!("`{}` does not take children", node.name().value())).label("unexpected block"),
            )),
            None => Ok(()),
        }
    }

    /// Checks argument count and property names; returns the arguments.
    fn check_entries<'n>(&mut self, node: &'n KdlNode, min: usize, max: usize, props: &[&str]) -> R<Vec<&'n KdlEntry>> {
        let name = node.name().value();
        let mut args = Vec::new();
        let mut seen: Vec<&str> = Vec::new();
        let mut ok = true;
        for e in node.entries() {
            match e.name() {
                None => {
                    if args.len() == max {
                        self.err(Diag::error(e.span(), format!("too many arguments for `{name}`")).label("unexpected"));
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
                        self.err(Diag::error(e.span(), format!("unknown property `{k}`")).label("unknown").help(help));
                        ok = false;
                    } else if seen.contains(&k) {
                        self.err(Diag::error(e.span(), format!("`{k}` given more than once")).label("duplicate"));
                        ok = false;
                    }
                    seen.push(k);
                }
            }
        }
        if args.len() < min {
            let what = if min == max { format!("{min}") } else { format!("at least {min}") };
            self.err(
                Diag::error(node.name().span(), format!("`{name}` needs {what} argument(s)"))
                    .label(format!("has {}", args.len())),
            );
            ok = false;
        }
        if ok { Ok(args) } else { Err(Failed) }
    }
}

fn unknown_node(node: &KdlNode, allowed: &[&str]) -> Diag {
    Diag::error(node.name().span(), format!("unknown node `{}`", node.name().value()))
        .label("unknown")
        .help(format!("expected one of: {}", allowed.join(", ")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn errors(src: &str) -> Vec<String> {
        let p = parse(src).expect("syntax");
        p.diagnostics.iter().filter(|d| d.severity == Severity::Error).map(|d| d.message.clone()).collect()
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
        let p = parse(src).unwrap();
        assert!(p.diagnostics.is_empty(), "{:?}", p.diagnostics);
        let s = p.scene.unwrap();
        assert_eq!((s.width, s.height), (800, 600));
        assert_eq!(s.paths.len(), 2);
        assert!(!s.paths[0].closed && s.paths[1].closed);
        assert_eq!(s.paths[0].gradients[1].stops[1].color.a, 0.0);
        assert_eq!(s.paths[1].points[2], [400.0, 450.0]);
    }

    #[test]
    fn errors_are_reported() {
        assert_eq!(errors("path { }"), ["`path` before `image`", "a path needs at least 2 points"]);
        assert_eq!(errors("image width=0 height=5"), ["`width` must be a whole number from 1 to 2147483647"]);
        assert_eq!(errors("image width=5 height=5 depth=3"), ["unknown property `depth`"]);
        assert_eq!(errors("image width=5 height=5\nfoo"), ["unknown node `foo`"]);
        assert_eq!(
            errors("image width=5 height=5\npath { point 0 0; point 1 1; gradient 4 { stop 0 0 0 0 } }"),
            ["on an open path, `t` must be in [0, 4)"]
        );
        assert_eq!(
            errors("image width=5 height=5\npath { point 0 0; point 1 1; gradient 1 { stop 2 0 0 0; stop 1 0 0 0 } }"),
            ["stop distances must not decrease"]
        );
        assert_eq!(
            errors("image width=5 height=5\npath { point 0 0; point 1 1; gradient 1 { stop 0 1.5 0 0 2 } }"),
            ["`L` must be from 0 to 1"]
        );
        assert_eq!(errors("image width=5 height=5\npath { point 0; point 1 1 }"), ["`point` needs 2 argument(s)"]);
        assert_eq!(errors("image width=5 height=5\npath { point 0 0 1; point 1 1 }"), ["too many arguments for `point`"]);
        assert_eq!(errors("image width=5 height=5\npath { point 0 0; point 1 1; gradient 1 }"), ["a gradient needs at least one stop"]);
    }

    #[test]
    fn closed_gradients_outside_sides_warn() {
        let p = parse("image width=5 height=5\npath closed=#true { point 0 0; point 1 1; gradient 2.5 { stop 0 0 0 0 } }").unwrap();
        let msgs: Vec<_> = p.diagnostics.iter().map(|d| (d.severity, d.message.as_str())).collect();
        assert_eq!(msgs.len(), 2);
        assert!(msgs.iter().all(|(s, _)| *s == Severity::Warning));
        assert!(p.scene.unwrap().paths[0].gradients.is_empty());
    }
}
