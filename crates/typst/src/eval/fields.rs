use ecow::{eco_format, EcoString};

use crate::diag::{At, Hint, SourceResult, StrResult};
use crate::geom::{
    Axes, Em, GenAlign, HorizontalAlign, Length, PartialStroke, Scalar, Smart, Stroke,
    VerticalAlign,
};
use crate::syntax::Span;

use super::{Dynamic, IntoValue, Value};

/// Try to access a field on a value.
/// This function is exclusively for types which have
/// predefined fields, such as stroke and length.
pub(crate) fn field(value: &Value, field: &str) -> StrResult<Value> {
    let name = value.type_name();
    let not_supported = || Err(no_fields(name));
    let missing = || Err(missing_field(name, field));

    // Special cases, such as module and dict, are handled by Value itself
    let result = match value {
        Value::Length(length) => match field {
            "em" => length.em.into_value(),
            "abs" => length.abs.into_value(),
            _ => return missing(),
        },
        Value::Relative(rel) => match field {
            "ratio" => rel.rel.into_value(),
            "length" => rel.abs.into_value(),
            _ => return missing(),
        },
        Value::Dyn(dynamic) => {
            if let Some(stroke) = dynamic.downcast::<PartialStroke>() {
                match field {
                    "paint" => stroke
                        .paint
                        .clone()
                        .unwrap_or_else(|| Stroke::default().paint)
                        .into_value(),
                    "thickness" => stroke
                        .thickness
                        .unwrap_or_else(|| Stroke::default().thickness.into())
                        .into_value(),
                    "cap" => stroke
                        .line_cap
                        .unwrap_or_else(|| Stroke::default().line_cap)
                        .into_value(),
                    "join" => stroke
                        .line_join
                        .unwrap_or_else(|| Stroke::default().line_join)
                        .into_value(),
                    "dash" => stroke.dash_pattern.clone().unwrap_or(None).into_value(),
                    "miter-limit" => stroke
                        .miter_limit
                        .unwrap_or_else(|| Stroke::default().miter_limit)
                        .0
                        .into_value(),
                    _ => return missing(),
                }
            } else if let Some(align2d) = dynamic.downcast::<Axes<GenAlign>>() {
                match field {
                    "x" => align2d.x.into_value(),
                    "y" => align2d.y.into_value(),
                    _ => return missing(),
                }
            } else {
                return not_supported();
            }
        }
        _ => return not_supported(),
    };

    Ok(result)
}

/// Attempts to change the value of a field.
pub(crate) fn field_mut(
    value: &mut Value,
    field: &str,
    new_value: Value,
    span: Span,
) -> SourceResult<()> {
    let name = value.type_name();
    let not_supported = || Err(no_fields_mut(name)).at(span);
    let missing = || Err(missing_field(name, field)).at(span);

    // Special cases, such as module and dict, are already handled by eval/mod.rs
    match value {
        Value::Length(length) => match field {
            "em" => length.em = Em::new(new_value.cast().at(span)?),
            "abs" => {
                let new_length: Length = new_value.cast().at(span)?;

                if new_length.em != Em::zero() {
                    return Err(eco_format!("cannot assign a length with non-zero em units ({new_length:?}) to another length's 'abs' field"))
                        .hint("assign 'length.abs' instead to ignore its em component")
                        .at(span);
                }

                length.abs = new_length.abs;
            }
            _ => return missing(),
        },
        Value::Relative(rel) => match field {
            "ratio" => rel.rel = new_value.cast().at(span)?,
            "length" => rel.abs = new_value.cast().at(span)?,
            _ => return missing(),
        },
        Value::Dyn(dynamic) => {
            if let Some(stroke) = dynamic.downcast::<PartialStroke>() {
                // workaround to the absence of downcast_mut, which is not
                // simple to implement due to the lack of a 'Clone' bound in
                // 'dyn Bound' (used in Dynamic's only field)
                let mut new_stroke = stroke.clone();
                match field {
                    // wrap in Smart::Custom to avoid having 'auto'
                    // accidentally be a valid value here
                    // (would be inconsistent with the constructor, at least)
                    "paint" => {
                        new_stroke.paint = Smart::Custom(new_value.cast().at(span)?)
                    }
                    "thickness" => {
                        new_stroke.thickness = Smart::Custom(new_value.cast().at(span)?)
                    }
                    "cap" => {
                        new_stroke.line_cap = Smart::Custom(new_value.cast().at(span)?)
                    }
                    "join" => {
                        new_stroke.line_join = Smart::Custom(new_value.cast().at(span)?)
                    }
                    "dash" => {
                        new_stroke.dash_pattern =
                            Smart::Custom(new_value.cast().at(span)?)
                    }
                    "miter-limit" => {
                        new_stroke.miter_limit =
                            Smart::Custom(Scalar(new_value.cast().at(span)?))
                    }
                    _ => return missing(),
                }
                *dynamic = Dynamic::new(new_stroke);
            } else if let Some(align2d) = dynamic.downcast::<Axes<GenAlign>>() {
                let mut new_align2d = *align2d;
                match field {
                    "x" => {
                        new_align2d.x =
                            new_value.cast::<HorizontalAlign>().at(span)?.into()
                    }
                    "y" => {
                        new_align2d.y = new_value.cast::<VerticalAlign>().at(span)?.into()
                    }
                    _ => return missing(),
                }
                *dynamic = Dynamic::new(new_align2d);
            } else {
                return not_supported();
            }
        }
        _ => return not_supported(),
    };

    Ok(())
}

/// The error message for a type not supporting field access.
#[cold]
fn no_fields(type_name: &str) -> EcoString {
    eco_format!("cannot access fields on type {type_name}")
}

/// The error message for a type not supporting field access ('field_mut'
/// variant).
#[cold]
fn no_fields_mut(type_name: &str) -> EcoString {
    eco_format!("{type_name} does not have accessible fields")
}

/// The missing field error message.
#[cold]
fn missing_field(type_name: &str, field: &str) -> EcoString {
    eco_format!("{type_name} does not contain field \"{field}\"")
}

/// List the available fields for a type.
pub fn fields_on(type_name: &str) -> &[&'static str] {
    match type_name {
        "length" => &["em", "abs"],
        "relative length" => &["ratio", "length"],
        "stroke" => &["paint", "thickness", "cap", "join", "dash", "miter-limit"],
        "2d alignment" => &["x", "y"],
        _ => &[],
    }
}
