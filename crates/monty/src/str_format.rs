//! `str.format`: the replacement fields of PEP 3101.
//!
//! The mini-language a spec is written in already exists, because an f-string
//! needs it. What a template adds is everything around the spec: which argument
//! a field names, what it reads out of that argument, and how the result is
//! converted before the spec applies. So a template is parsed here at run time,
//! each field is resolved against the arguments the call was given, and the
//! value and its spec go to the same formatter an f-string uses.
//!
//! The one thing a template can do that an f-string cannot is carry its fields
//! as data: an f-string's are fixed where it is written, and a template's are
//! chosen when it is applied. That is the whole reason to have both.

use crate::{
    bytecode::{CallResult, VM},
    exception_private::{ExcType, RunError, RunResult, SimpleException},
    fstring::{ParseFormatSpecError, ascii_escape, format_with_spec},
    types::{PyTrait, str::allocate_string},
    value::{EitherStr, Value},
};

/// How deep a spec may nest its own fields.
///
/// CPython counts the template as the first level, so a spec may hold fields
/// and those fields may not. The error it raises when they do is this one.
const DEPTH: u8 = 2;

/// Formats `template` against the arguments a call supplied.
///
/// `args` and `kwargs` stay the caller's: every value read out of them is
/// cloned before it is used and released here, so nothing this does changes
/// what the caller still has to drop.
pub(crate) fn format_template(
    template: &str,
    args: &[Value],
    kwargs: &[(String, Value)],
    vm: &mut VM<'_>,
) -> RunResult<String> {
    let mut state = Numbering::Untouched;
    render(template, args, kwargs, &mut state, DEPTH, vm)
}

/// Which way this template numbers its positional fields.
///
/// Automatic and manual cannot be mixed, so the first field that says which
/// decides, and every later one is held to it.
enum Numbering {
    Untouched,
    /// Automatic: the next field with no name takes this index.
    Automatic(usize),
    Manual,
}

impl Numbering {
    /// The index an unnamed field takes, or a refusal if this template has
    /// already numbered one by hand.
    fn next(&mut self) -> RunResult<usize> {
        match self {
            Self::Manual => Err(value_error(
                "cannot switch from manual field specification to automatic field numbering",
            )),
            Self::Untouched => {
                *self = Self::Automatic(1);
                Ok(0)
            }
            Self::Automatic(at) => {
                let taken = *at;
                *at += 1;
                Ok(taken)
            }
        }
    }

    /// Records that a field named its own index.
    fn by_hand(&mut self) -> RunResult<()> {
        if let Self::Automatic(_) = self {
            return Err(value_error(
                "cannot switch from automatic field numbering to manual field specification",
            ));
        }
        *self = Self::Manual;
        Ok(())
    }
}

/// One pass over a template, which a nested spec re-enters with one less depth.
fn render(
    template: &str,
    args: &[Value],
    kwargs: &[(String, Value)],
    numbering: &mut Numbering,
    depth: u8,
    vm: &mut VM<'_>,
) -> RunResult<String> {
    let mut out = String::new();
    let bytes = template.as_bytes();
    let mut at = 0;
    while at < bytes.len() {
        let here = bytes[at];
        if here != b'{' && here != b'}' {
            let from = at;
            while at < bytes.len() && bytes[at] != b'{' && bytes[at] != b'}' {
                at += 1;
            }
            out.push_str(&template[from..at]);
            continue;
        }
        // A doubled brace is the only way to write one.
        if bytes.get(at + 1) == Some(&here) {
            out.push(char::from(here));
            at += 2;
            continue;
        }
        if here == b'}' {
            return Err(value_error("Single '}' encountered in format string"));
        }
        let (field, next) = whole_field(template, at)?;
        at = next;
        out.push_str(&filled(field, args, kwargs, numbering, depth, vm)?);
    }
    Ok(out)
}

/// The text of the field beginning at `open`, and where the template carries on.
///
/// A spec may hold fields of its own, so the closing brace is the one that
/// matches rather than the first one seen.
fn whole_field(template: &str, open: usize) -> RunResult<(&str, usize)> {
    let bytes = template.as_bytes();
    let mut depth = 1;
    let mut at = open + 1;
    while at < bytes.len() {
        match bytes[at] {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Ok((&template[open + 1..at], at + 1));
                }
            }
            _ => {}
        }
        at += 1;
    }
    Err(value_error("expected '}' before end of string"))
}

/// One field, resolved and formatted.
fn filled(
    field: &str,
    args: &[Value],
    kwargs: &[(String, Value)],
    numbering: &mut Numbering,
    depth: u8,
    vm: &mut VM<'_>,
) -> RunResult<String> {
    let (name, conversion, spec) = split_field(field)?;
    let value = resolve(name, args, kwargs, numbering, vm)?;

    let converted = match conversion {
        None => None,
        Some('s') => Some(text_of(value.py_str(vm)?, vm)?),
        Some('r') => Some(text_of(value.py_repr(vm)?, vm)?),
        Some('a') => Some(ascii_escape(&text_of(value.py_repr(vm)?, vm)?)),
        Some(other) => {
            value.drop_with(vm);
            return Err(value_error(format!("Unknown conversion specifier {other}")));
        }
    };

    // A spec of its own may name fields, and those are resolved against the
    // same arguments before the spec is read as a spec.
    let spec = match spec {
        Some(spec) if spec.contains('{') => {
            if depth <= 1 {
                value.drop_with(vm);
                return Err(value_error("Max string recursion exceeded"));
            }
            Some(render(spec, args, kwargs, numbering, depth - 1, vm)?)
        }
        Some(spec) => Some(spec.to_owned()),
        None => None,
    };

    // A converted value is a string from here on, exactly as an f-string's is.
    let subject = match converted {
        Some(said) => {
            value.drop_with(vm);
            allocate_string(said, vm.heap)
        }
        None => value,
    };
    let out = formatted(&subject, spec.as_deref(), vm);
    subject.drop_with(vm);
    out
}

/// A value and its spec, through the formatter an f-string uses.
fn formatted(value: &Value, spec: Option<&str>, vm: &mut VM<'_>) -> RunResult<String> {
    let Some(spec) = spec else {
        return text_of(value.py_str(vm)?, vm);
    };
    let parsed = spec.parse().map_err(|err: ParseFormatSpecError| {
        let message = if err.needs_type_suffix() {
            let named = value.py_type_name(vm);
            format!("{err} for object of type '{named}'")
        } else {
            err.to_string()
        };
        RunError::Exc(SimpleException::new_msg(ExcType::ValueError, message).into())
    })?;
    format_with_spec(value, &parsed, vm)
}

/// A field's three parts: what it names, how it is converted, and its spec.
///
/// The name ends at the first `!` or `:` outside a subscript, since a key may
/// hold either character and means nothing by it.
fn split_field(field: &str) -> RunResult<(&str, Option<char>, Option<&str>)> {
    let bytes = field.as_bytes();
    let mut inside = false;
    for (at, byte) in bytes.iter().enumerate() {
        match byte {
            b'[' => inside = true,
            b']' => inside = false,
            b'!' if !inside => {
                let rest = &field[at + 1..];
                let mut said = rest.chars();
                let Some(how) = said.next() else {
                    return Err(value_error("end of string while looking for conversion specifier"));
                };
                let after = &rest[how.len_utf8()..];
                return match after.strip_prefix(':') {
                    Some(spec) => Ok((&field[..at], Some(how), Some(spec))),
                    None if after.is_empty() => Ok((&field[..at], Some(how), None)),
                    None => Err(value_error("expected ':' after conversion specifier")),
                };
            }
            b':' if !inside => return Ok((&field[..at], None, Some(&field[at + 1..]))),
            _ => {}
        }
    }
    Ok((field, None, None))
}

/// The value a field name stands for, with everything it reads out of it.
fn resolve(
    name: &str,
    args: &[Value],
    kwargs: &[(String, Value)],
    numbering: &mut Numbering,
    vm: &mut VM<'_>,
) -> RunResult<Value> {
    let end = name.find(['.', '[']).unwrap_or(name.len());
    let (first, mut rest) = name.split_at(end);
    let base = argument(first, args, kwargs, numbering, vm)?;

    let mut held = base.clone_with_heap(vm);
    while !rest.is_empty() {
        let next = if let Some(after) = rest.strip_prefix('.') {
            let end = after.find(['.', '[']).unwrap_or(after.len());
            let (attr, tail) = after.split_at(end);
            if attr.is_empty() {
                held.drop_with(vm);
                return Err(value_error("Empty attribute in format string"));
            }
            rest = tail;
            let read = held.py_getattr(&EitherStr::from(attr.to_owned()), vm);
            match read {
                Ok(CallResult::Value(got)) => got,
                // A read that answers with a frame to run rather than a value.
                // Nothing reaches this today, since a descriptor is resolved
                // where the attribute is read; it stands so that a read which
                // one day needs to suspend says so instead of being dropped.
                Ok(_) => {
                    held.drop_with(vm);
                    return Err(RunError::Exc(
                        SimpleException::new_msg(
                            ExcType::NotImplementedError,
                            format!("reading '{attr}' in a format field would have to run code, which it cannot here"),
                        )
                        .into(),
                    ));
                }
                Err(e) => {
                    held.drop_with(vm);
                    return Err(e);
                }
            }
        } else {
            let after = &rest[1..];
            let Some(shut) = after.find(']') else {
                held.drop_with(vm);
                return Err(value_error("Missing ']' in format string"));
            };
            let (key, tail) = after.split_at(shut);
            rest = &tail[1..];
            // A key of digits is an index, as CPython reads it; anything else
            // is the string it looks like.
            let key = if !key.is_empty() && key.bytes().all(|b| b.is_ascii_digit()) {
                match key.parse::<i64>() {
                    Ok(at) => Value::Int(at),
                    Err(_) => allocate_string(key.to_owned(), vm.heap),
                }
            } else {
                allocate_string(key.to_owned(), vm.heap)
            };
            let read = held.py_getitem(&key, vm);
            key.drop_with(vm);
            match read {
                Ok(got) => got,
                Err(e) => {
                    held.drop_with(vm);
                    return Err(e);
                }
            }
        };
        held.drop_with(vm);
        held = next;
    }
    Ok(held)
}

/// Which argument a field's first part names.
///
/// Borrowed rather than cloned: what the caller supplied is still the
/// caller's, and the walk above takes its own reference before reading.
fn argument<'a>(
    first: &str,
    args: &'a [Value],
    kwargs: &'a [(String, Value)],
    numbering: &mut Numbering,
    vm: &mut VM<'_>,
) -> RunResult<&'a Value> {
    if first.is_empty() {
        let at = numbering.next()?;
        return args.get(at).ok_or_else(|| {
            RunError::Exc(
                SimpleException::new_msg(
                    ExcType::IndexError,
                    format!("Replacement index {at} out of range for positional args tuple"),
                )
                .into(),
            )
        });
    }
    if first.bytes().all(|b| b.is_ascii_digit()) {
        numbering.by_hand()?;
        let at: usize = first
            .parse()
            .map_err(|_| value_error(format!("Replacement index {first} is too large")))?;
        return args.get(at).ok_or_else(|| {
            RunError::Exc(
                SimpleException::new_msg(
                    ExcType::IndexError,
                    format!("Replacement index {at} out of range for positional args tuple"),
                )
                .into(),
            )
        });
    }
    let _ = vm;
    kwargs
        .iter()
        .find(|(name, _)| name == first)
        .map(|(_, value)| value)
        .ok_or_else(|| RunError::Exc(SimpleException::new_msg(ExcType::KeyError, format!("'{first}'")).into()))
}

/// A `str` value, as owned text, releasing the value either way.
fn text_of(value: Value, vm: &mut VM<'_>) -> RunResult<String> {
    let said = value.to_str(vm).map(str::to_owned);
    value.drop_with(vm);
    said
}

fn value_error(message: impl Into<String>) -> RunError {
    RunError::Exc(SimpleException::new_msg(ExcType::ValueError, message.into()).into())
}
