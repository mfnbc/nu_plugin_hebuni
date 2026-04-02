use nu_plugin::{
    serve_plugin, EngineInterface, EvaluatedCall, MsgPackSerializer, Plugin, PluginCommand,
    SimplePluginCommand,
};
use nu_protocol::{Category, LabeledError, Record, Signature, Span, SyntaxShape, Type, Value};
use std::collections::HashSet;
use std::iter::once;
use unicode_normalization::UnicodeNormalization;

// ── Plugin shell ──────────────────────────────────────────────────────────────

struct HebuniPlugin;

impl Plugin for HebuniPlugin {
    fn version(&self) -> String {
        env!("CARGO_PKG_VERSION").into()
    }

    fn commands(&self) -> Vec<Box<dyn PluginCommand<Plugin = Self>>> {
        vec![Box::new(ScalarStrip::new()), Box::new(Recompose::new())]
    }
}

// ── hebuni scalar-strip ───────────────────────────────────────────────────────

struct ScalarStrip;

impl ScalarStrip {
    fn new() -> Self {
        Self
    }
}

impl SimplePluginCommand for ScalarStrip {
    type Plugin = HebuniPlugin;

    fn name(&self) -> &str {
        "hebuni scalar-strip"
    }

    fn signature(&self) -> Signature {
        Signature::build(PluginCommand::name(self))
            .input_output_types(vec![(Type::Any, Type::Record(vec![].into()))])
            .required(
                "surface",
                SyntaxShape::String,
                "The pointed Hebrew surface form to analyse",
            )
            .category(Category::Custom("hebuni".into()))
    }

    fn description(&self) -> &str {
        "Walk a pointed Hebrew surface form and emit its scalar substrate."
    }

    fn extra_description(&self) -> &str {
        "Returns a record with four fields:
  surface    — the original input string, unchanged
  consonants — only U+05D0–U+05EA scalars joined into a string, in stream order
  nfc_chars  — every scalar NFC-normalised, as a flat list of single-char strings
  stripped   — non-consonant scalars only, each as a record:
                 { abs_pos, byte_pos, after_cons, ch, cp, cp_hex }

Rust does no Hebrew classification.  Every scalar outside U+05D0–U+05EA
goes into stripped regardless of its meaning.  normalize.nu applies all rules."
    }

    fn run(
        &self,
        _plugin: &HebuniPlugin,
        _engine: &EngineInterface,
        call: &EvaluatedCall,
        _input: &Value,
    ) -> Result<Value, LabeledError> {
        let span = call.head;
        let surface: String = call.req(0)?;
        scalar_strip(&surface, span)
            .map_err(|e| LabeledError::new(e).with_label("scalar-strip failed", span))
    }
}

// ── hebuni recompose ──────────────────────────────────────────────────────────

struct Recompose;

impl Recompose {
    fn new() -> Self {
        Self
    }
}

impl SimplePluginCommand for Recompose {
    type Plugin = HebuniPlugin;

    fn name(&self) -> &str {
        "hebuni recompose"
    }

    fn signature(&self) -> Signature {
        Signature::build(PluginCommand::name(self))
            .input_output_types(vec![(Type::Any, Type::String)])
            .required(
                "nfc_chars",
                SyntaxShape::List(Box::new(SyntaxShape::String)),
                "Flat list of NFC single-char strings from scalar-strip",
            )
            .required(
                "keep_indices",
                SyntaxShape::List(Box::new(SyntaxShape::Int)),
                "Indices into nfc_chars to keep, in ascending order",
            )
            .category(Category::Custom("hebuni".into()))
    }

    fn description(&self) -> &str {
        "Filter nfc_chars by keep_indices, join, and return a canonical NFC string."
    }

    fn extra_description(&self) -> &str {
        "Hebrew-specific recomposer helper.
Given a flat list of NFC scalar strings and a list of indices to keep,
filters to only those scalars, joins them in order, applies a final NFC
normalization pass, and returns the result.

The command itself does not infer Hebrew structure; the caller decides
which scalars belong in the structural form."
    }

    fn run(
        &self,
        _plugin: &HebuniPlugin,
        _engine: &EngineInterface,
        call: &EvaluatedCall,
        _input: &Value,
    ) -> Result<Value, LabeledError> {
        let span = call.head;

        let nfc_chars_val: Value = call.req(0)?;
        let keep_indices_val: Value = call.req(1)?;

        let nfc_chars: Vec<String> = match nfc_chars_val {
            Value::List { vals, .. } => vals
                .into_iter()
                .map(|v| v.as_str().map(|s| s.to_owned()))
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| {
                    LabeledError::new(e.to_string())
                        .with_label("nfc_chars must be list<string>", span)
                })?,
            other => {
                return Err(
                    LabeledError::new(format!("expected list, got {:?}", other.get_type()))
                        .with_label("nfc_chars must be a list<string>", span),
                )
            }
        };

        let keep_indices: HashSet<usize> = match keep_indices_val {
            Value::List { vals, .. } => vals
                .into_iter()
                .map(|v| v.as_int().map(|i| i as usize))
                .collect::<Result<HashSet<_>, _>>()
                .map_err(|e| {
                    LabeledError::new(e.to_string())
                        .with_label("keep_indices must be list<int>", span)
                })?,
            other => {
                return Err(
                    LabeledError::new(format!("expected list, got {:?}", other.get_type()))
                        .with_label("keep_indices must be a list<int>", span),
                )
            }
        };

        let result = recompose(&nfc_chars, &keep_indices);
        Ok(Value::string(result, span))
    }
}

// ── Core logic ────────────────────────────────────────────────────────────────

fn scalar_strip(surface: &str, span: Span) -> Result<Value, String> {
    let mut consonants = String::new();
    let mut nfc_chars: Vec<Value> = Vec::new();
    let mut stripped: Vec<Value> = Vec::new();

    let mut abs_pos: i64 = 0;
    let mut after_cons: i64 = 0;

    for (nfc_char, byte_pos) in surface
        .char_indices()
        .map(|(byte_off, ch)| (ch, byte_off as i64))
    {
        let nfd_scalars: Vec<char> = once(nfc_char).nfd().collect();

        for nfd_scalar in nfd_scalars {
            let cp = nfd_scalar as u32;
            let nfc_form: String = once(nfd_scalar).nfc().collect();

            nfc_chars.push(Value::string(nfc_form.clone(), span));

            if is_hebrew_consonant(cp) {
                consonants.push(nfd_scalar);
                after_cons += 1;
            } else {
                let cp_hex = format!("{:04X}", cp);
                stripped.push(make_stripped_record(
                    abs_pos, byte_pos, after_cons, &nfc_form, cp, &cp_hex, span,
                ));
            }

            abs_pos += 1;
        }
    }

    let mut rec = Record::new();
    rec.insert("surface", Value::string(surface, span));
    rec.insert("consonants", Value::string(consonants, span));
    rec.insert("nfc_chars", Value::list(nfc_chars, span));
    rec.insert("stripped", Value::list(stripped, span));

    Ok(Value::record(rec, span))
}

/// Filter nfc_chars by keep_indices, join, and NFC-normalize the result.
/// No Hebrew knowledge — pure Unicode recomposition.
fn recompose(nfc_chars: &[String], keep_indices: &HashSet<usize>) -> String {
    let filtered: String = nfc_chars
        .iter()
        .enumerate()
        .filter(|(i, _)| keep_indices.contains(i))
        .map(|(_, ch)| ch.as_str())
        .collect();

    filtered.chars().nfc().collect()
}

// ── Helpers ───────────────────────────────────────────────────────────────────

#[inline(always)]
fn is_hebrew_consonant(cp: u32) -> bool {
    (0x05D0..=0x05EA).contains(&cp)
}

fn make_stripped_record(
    abs_pos: i64,
    byte_pos: i64,
    after_cons: i64,
    ch: &str,
    cp: u32,
    cp_hex: &str,
    span: Span,
) -> Value {
    let mut rec = Record::new();
    rec.insert("abs_pos", Value::int(abs_pos, span));
    rec.insert("byte_pos", Value::int(byte_pos, span));
    rec.insert("after_cons", Value::int(after_cons, span));
    rec.insert("ch", Value::string(ch, span));
    rec.insert("cp", Value::int(cp as i64, span));
    rec.insert("cp_hex", Value::string(cp_hex, span));
    Value::record(rec, span)
}

// ── Entry point ───────────────────────────────────────────────────────────────

fn main() {
    serve_plugin(&HebuniPlugin, MsgPackSerializer);
}
