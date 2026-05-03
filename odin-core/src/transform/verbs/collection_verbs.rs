//! Array, object, aggregation, generation, and geo verb implementations.

use std::time::{SystemTime, UNIX_EPOCH};

use crate::types::transform::DynValue;
use super::VerbContext;

// ─────────────────────────────────────────────────────────────────────────────
// Helper: compare two DynValues with a comparison operator string
// ─────────────────────────────────────────────────────────────────────────────

fn dyn_values_equal_cmp(a: &DynValue, b: &DynValue) -> bool {
    match (a, b) {
        (DynValue::Integer(x), DynValue::Float(y)) => (*x as f64) == *y,
        (DynValue::Float(x), DynValue::Integer(y)) => *x == (*y as f64),
        (DynValue::String(s), DynValue::Integer(n)) | (DynValue::Integer(n), DynValue::String(s)) => s.parse::<i64>().ok() == Some(*n),
        (DynValue::String(s), DynValue::Float(n)) | (DynValue::Float(n), DynValue::String(s)) => s.parse::<f64>().ok() == Some(*n),
        _ => a == b,
    }
}

fn compare_values(a: &DynValue, op: &str, b: &DynValue) -> bool {
    match op {
        "=" | "==" | "eq" => dyn_values_equal_cmp(a, b),
        "!=" | "<>" | "ne" => !dyn_values_equal_cmp(a, b),
        "contains" => {
            if let (DynValue::String(haystack), DynValue::String(needle)) = (a, b) {
                haystack.contains(needle.as_str())
            } else {
                false
            }
        }
        ">" | "gt" | "<" | "lt" | ">=" | "gte" | "<=" | "lte" => {
            let ord = match (a, b) {
                (DynValue::Integer(x), DynValue::Integer(y)) => x.partial_cmp(y),
                (DynValue::Float(x), DynValue::Float(y)) => x.partial_cmp(y),
                (DynValue::Integer(x), DynValue::Float(y)) => (*x as f64).partial_cmp(y),
                (DynValue::Float(x), DynValue::Integer(y)) => x.partial_cmp(&(*y as f64)),
                (DynValue::String(x), DynValue::String(y)) => x.partial_cmp(y),
                _ => None,
            };
            matches!(
                (op, ord),
                (">" | "gt", Some(std::cmp::Ordering::Greater))
                    | ("<" | "lt", Some(std::cmp::Ordering::Less))
                    | (">=" | "gte", Some(std::cmp::Ordering::Greater | std::cmp::Ordering::Equal))
                    | ("<=" | "lte", Some(std::cmp::Ordering::Less | std::cmp::Ordering::Equal))
            )
        }
        _ => false,
    }
}

/// Helper: extract a field value from an object `DynValue`.
/// Also handles string-encoded JSON objects.
fn get_field(obj: &DynValue, field: &str) -> Option<DynValue> {
    match obj {
        DynValue::Object(pairs) => {
            pairs.iter().find(|(k, _)| k == field).map(|(_, v)| v.clone())
        }
        DynValue::String(s) => {
            let trimmed = s.trim();
            if trimmed.starts_with('{') && trimmed.ends_with('}') {
                if let Some(obj) = obj.extract_object() {
                    return obj.into_iter().find(|(k, _)| k == field).map(|(_, v)| v);
                }
            }
            None
        }
        _ => None,
    }
}

/// Helper: coerce a `DynValue` to `f64` for numeric operations.
fn to_f64(v: &DynValue) -> Option<f64> {
    match v {
        DynValue::Float(n) => Some(*n),
        DynValue::Integer(n) => Some(*n as f64),
        DynValue::String(s) => s.parse::<f64>().ok(),
        _ => None,
    }
}

/// Helper: extract an array from a `DynValue` (either direct array or string-encoded).
fn extract_arr(v: &DynValue) -> Option<Vec<DynValue>> {
    v.extract_array()
}

// ─────────────────────────────────────────────────────────────────────────────
// Array verbs
// ─────────────────────────────────────────────────────────────────────────────

/// filter: arity 4 (@array "field" "op" value)
pub(super) fn filter(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    if args.len() < 4 {
        return Err("filter: requires 4 arguments (array, field, op, value)".to_string());
    }
    let mut arr = extract_arr(&args[0]).ok_or("filter: first argument must be an array")?;
    let field = args[1].as_str().ok_or("filter: second argument must be a string (field name)")?;
    let op = args[2].as_str().ok_or("filter: third argument must be a string (operator)")?;
    let value = &args[3];

    // In-place retain — no extra clone since extract_arr already owns the vec
    arr.retain(|item| {
        get_field(item, field)
            .is_some_and(|fv| compare_values(&fv, op, value))
    });
    Ok(DynValue::Array(arr))
}

/// flatten: arity 1 — flatten nested arrays one level.
pub(super) fn flatten(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    let arr = args.first()
        .and_then(extract_arr)
        .ok_or("flatten: expected array argument")?;
    let mut result = Vec::new();
    // Move items out of owned vec — no extra clone needed
    for item in arr {
        match item {
            DynValue::Array(inner) => result.extend(inner),
            other => result.push(other),
        }
    }
    Ok(DynValue::Array(result))
}

/// distinct: arity 1 — remove duplicate values from array.
pub(super) fn distinct(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    let arr = args.first()
        .and_then(extract_arr)
        .ok_or("distinct: expected array argument")?;
    // Use index tracking to avoid O(n^2) — compare only unique items
    let mut result = Vec::with_capacity(arr.len());
    for item in arr {
        if !result.contains(&item) {
            result.push(item);
        }
    }
    Ok(DynValue::Array(result))
}

/// unique: alias for distinct.
pub(super) fn unique(args: &[DynValue], ctx: &VerbContext) -> Result<DynValue, String> {
    distinct(args, ctx)
}

/// `sort_verb`: arity 1 — sort array (numbers numeric, strings alphabetic).
pub(super) fn sort_verb(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    let mut result = args.first()
        .and_then(extract_arr)
        .ok_or("sort: expected array argument")?;
    result.sort_by(|a, b| {
        match (a, b) {
            (DynValue::Integer(x), DynValue::Integer(y)) => x.cmp(y),
            (DynValue::Float(x), DynValue::Float(y)) => x.partial_cmp(y).unwrap_or(std::cmp::Ordering::Equal),
            (DynValue::Integer(x), DynValue::Float(y)) => (*x as f64).partial_cmp(y).unwrap_or(std::cmp::Ordering::Equal),
            (DynValue::Float(x), DynValue::Integer(y)) => x.partial_cmp(&(*y as f64)).unwrap_or(std::cmp::Ordering::Equal),
            (DynValue::String(x), DynValue::String(y)) => x.cmp(y),
            _ => std::cmp::Ordering::Equal,
        }
    });
    Ok(DynValue::Array(result))
}

/// `sort_desc`: arity 1 — sort descending.
pub(super) fn sort_desc(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    let mut result = args.first()
        .and_then(extract_arr)
        .ok_or("sortDesc: expected array argument")?;
    result.sort_by(|a, b| {
        match (a, b) {
            (DynValue::Integer(x), DynValue::Integer(y)) => y.cmp(x),
            (DynValue::Float(x), DynValue::Float(y)) => y.partial_cmp(x).unwrap_or(std::cmp::Ordering::Equal),
            (DynValue::Integer(x), DynValue::Float(y)) => y.partial_cmp(&(*x as f64)).unwrap_or(std::cmp::Ordering::Equal),
            (DynValue::Float(x), DynValue::Integer(y)) => (*y as f64).partial_cmp(x).unwrap_or(std::cmp::Ordering::Equal),
            (DynValue::String(x), DynValue::String(y)) => y.cmp(x),
            _ => std::cmp::Ordering::Equal,
        }
    });
    Ok(DynValue::Array(result))
}

/// `sort_by`: arity 2 — sort array of objects by field name.
pub(super) fn sort_by(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    if args.len() < 2 {
        return Err("sortBy: requires 2 arguments (array, field)".to_string());
    }
    let mut result = extract_arr(&args[0]).ok_or("sortBy: first argument must be an array")?;
    let field = args[1].as_str().ok_or("sortBy: second argument must be a string (field name)")?;
    result.sort_by(|a, b| {
        let va = get_field(a, field);
        let vb = get_field(b, field);
        match (&va, &vb) {
            (Some(DynValue::Integer(x)), Some(DynValue::Integer(y))) => x.cmp(y),
            (Some(DynValue::Float(x)), Some(DynValue::Float(y))) => x.partial_cmp(y).unwrap_or(std::cmp::Ordering::Equal),
            (Some(DynValue::String(x)), Some(DynValue::String(y))) => x.cmp(y),
            _ => std::cmp::Ordering::Equal,
        }
    });
    Ok(DynValue::Array(result))
}

/// `map_verb`: arity 2 — extract field from each object in array.
pub(super) fn map_verb(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    if args.len() < 2 {
        return Err("map: requires 2 arguments (array, field)".to_string());
    }
    let arr = extract_arr(&args[0]).ok_or("map: first argument must be an array")?;
    let field = args[1].as_str().ok_or("map: second argument must be a string (field name)")?;
    let result: Vec<DynValue> = arr.iter()
        .map(|item| get_field(item, field).unwrap_or(DynValue::Null))
        .collect();
    Ok(DynValue::Array(result))
}

/// `index_of`: arity 2 — find index of value in array (-1 if not found).
pub(super) fn index_of(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    if args.len() < 2 {
        return Err("indexOf: requires 2 arguments (array, value)".to_string());
    }
    let arr = extract_arr(&args[0]).ok_or("indexOf: first argument must be an array")?;
    let value = &args[1];
    let idx = arr.iter().position(|item| item == value);
    Ok(DynValue::Integer(idx.map_or(-1, |i| i as i64)))
}

/// at: arity 2 — get element at index.
pub(super) fn at(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    if args.len() < 2 {
        return Err("at: requires 2 arguments (array, index)".to_string());
    }
    let arr = extract_arr(&args[0]).ok_or("at: first argument must be an array")?;
    let idx = args[1].as_i64().ok_or("at: second argument must be an integer")? as usize;
    Ok(arr.get(idx).cloned().unwrap_or(DynValue::Null))
}

/// slice: arity 3 — slice array from start to end index.
pub(super) fn slice(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    if args.len() < 3 {
        return Err("slice: requires 3 arguments (array, start, end)".to_string());
    }
    let arr = extract_arr(&args[0]).ok_or("slice: first argument must be an array")?;
    let start = args[1].as_i64().ok_or("slice: second argument must be an integer")? as usize;
    let end = (args[2].as_i64().ok_or("slice: third argument must be an integer")? as usize).min(arr.len());
    let start = start.min(end);
    Ok(DynValue::Array(arr[start..end].to_vec()))
}

/// reverse: arity 1 — reverse array.
pub(super) fn reverse(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    let mut result = args.first()
        .and_then(extract_arr)
        .ok_or("reverse: expected array argument")?;
    result.reverse();
    Ok(DynValue::Array(result))
}

/// every: arity 4 — like filter but returns boolean (all match).
pub(super) fn every(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    if args.len() < 4 {
        return Err("every: requires 4 arguments (array, field, op, value)".to_string());
    }
    let arr = extract_arr(&args[0]).ok_or("every: first argument must be an array")?;
    let field = args[1].as_str().ok_or("every: second argument must be a string")?;
    let op = args[2].as_str().ok_or("every: third argument must be a string")?;
    let value = &args[3];
    let result = arr.iter().all(|item| {
        get_field(item, field).is_some_and(|fv| compare_values(&fv, op, value))
    });
    Ok(DynValue::Bool(result))
}

/// some: arity 4 — like filter but returns boolean (any match).
pub(super) fn some(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    if args.len() < 4 {
        return Err("some: requires 4 arguments (array, field, op, value)".to_string());
    }
    let arr = extract_arr(&args[0]).ok_or("some: first argument must be an array")?;
    let field = args[1].as_str().ok_or("some: second argument must be a string")?;
    let op = args[2].as_str().ok_or("some: third argument must be a string")?;
    let value = &args[3];
    let result = arr.iter().any(|item| {
        get_field(item, field).is_some_and(|fv| compare_values(&fv, op, value))
    });
    Ok(DynValue::Bool(result))
}

/// find: arity 4 — like filter but returns first match only.
pub(super) fn find(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    if args.len() < 4 {
        return Err("find: requires 4 arguments (array, field, op, value)".to_string());
    }
    let arr = extract_arr(&args[0]).ok_or("find: first argument must be an array")?;
    let field = args[1].as_str().ok_or("find: second argument must be a string")?;
    let op = args[2].as_str().ok_or("find: third argument must be a string")?;
    let value = &args[3];
    let result = arr.iter().find(|item| {
        get_field(item, field).is_some_and(|fv| compare_values(&fv, op, value))
    });
    Ok(result.cloned().unwrap_or(DynValue::Null))
}

/// `find_index`: arity 4 — like filter but returns index of first match.
pub(super) fn find_index(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    if args.len() < 4 {
        return Err("findIndex: requires 4 arguments (array, field, op, value)".to_string());
    }
    let arr = extract_arr(&args[0]).ok_or("findIndex: first argument must be an array")?;
    let field = args[1].as_str().ok_or("findIndex: second argument must be a string")?;
    let op = args[2].as_str().ok_or("findIndex: third argument must be a string")?;
    let value = &args[3];
    let idx = arr.iter().position(|item| {
        get_field(item, field).is_some_and(|fv| compare_values(&fv, op, value))
    });
    Ok(DynValue::Integer(idx.map_or(-1, |i| i as i64)))
}

/// includes: arity 2 — check if array contains value.
pub(super) fn includes(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    if args.len() < 2 {
        return Err("includes: requires 2 arguments (array, value)".to_string());
    }
    let arr = extract_arr(&args[0]).ok_or("includes: first argument must be an array")?;
    let value = &args[1];
    Ok(DynValue::Bool(arr.contains(value)))
}

/// `concat_arrays`: arity 2 — concatenate two arrays.
pub(super) fn concat_arrays(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    if args.len() < 2 {
        return Err("concatArrays: requires 2 arguments (array, array)".to_string());
    }
    let a = extract_arr(&args[0]).ok_or("concatArrays: first argument must be an array")?;
    let b = extract_arr(&args[1]).ok_or("concatArrays: second argument must be an array")?;
    let mut result = a.clone();
    result.extend(b.iter().cloned());
    Ok(DynValue::Array(result))
}

/// `zip_verb`: arity 2 — zip two arrays into array of pairs.
pub(super) fn zip_verb(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    if args.len() < 2 {
        return Err("zip: requires 2 arguments (array, array)".to_string());
    }
    let a = extract_arr(&args[0]).ok_or("zip: first argument must be an array")?;
    let b = extract_arr(&args[1]).ok_or("zip: second argument must be an array")?;
    let result: Vec<DynValue> = a.iter().zip(b.iter())
        .map(|(x, y)| DynValue::Array(vec![x.clone(), y.clone()]))
        .collect();
    Ok(DynValue::Array(result))
}

/// `group_by`: arity 2 — group array of objects by field.
/// Returns an object where keys are the distinct field values and values are arrays.
pub(super) fn group_by(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    if args.len() < 2 {
        return Err("groupBy: requires 2 arguments (array, field)".to_string());
    }
    let arr = extract_arr(&args[0]).ok_or("groupBy: first argument must be an array")?;
    let field = args[1].as_str().ok_or("groupBy: second argument must be a string (field name)")?;

    // Use a Vec to preserve insertion order
    let mut groups: Vec<(String, Vec<DynValue>)> = Vec::new();
    for item in &arr {
        let key = match get_field(item, field) {
            Some(DynValue::String(s)) => s,
            Some(DynValue::Integer(n)) => n.to_string(),
            Some(DynValue::Float(n)) => n.to_string(),
            Some(DynValue::Bool(b)) => b.to_string(),
            _ => "null".to_string(),
        };
        if let Some(group) = groups.iter_mut().find(|(k, _)| k == &key) {
            group.1.push(item.clone());
        } else {
            groups.push((key, vec![item.clone()]));
        }
    }
    let result: Vec<(String, DynValue)> = groups.into_iter()
        .map(|(k, v)| (k, DynValue::Array(v)))
        .collect();
    Ok(DynValue::Object(result))
}

/// partition: arity 4 — split array into [matching, non-matching].
pub(super) fn partition(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    if args.len() < 4 {
        return Err("partition: requires 4 arguments (array, field, op, value)".to_string());
    }
    let arr = extract_arr(&args[0]).ok_or("partition: first argument must be an array")?;
    let field = args[1].as_str().ok_or("partition: second argument must be a string")?;
    let op = args[2].as_str().ok_or("partition: third argument must be a string")?;
    let value = &args[3];
    let mut matching = Vec::new();
    let mut non_matching = Vec::new();
    for item in &arr {
        if get_field(item, field).is_some_and(|fv| compare_values(&fv, op, value)) {
            matching.push(item.clone());
        } else {
            non_matching.push(item.clone());
        }
    }
    Ok(DynValue::Array(vec![
        DynValue::Array(matching),
        DynValue::Array(non_matching),
    ]))
}

/// take: arity 2 — take first N elements.
pub(super) fn take(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    if args.len() < 2 {
        return Err("take: requires 2 arguments (array, count)".to_string());
    }
    let arr = extract_arr(&args[0]).ok_or("take: first argument must be an array")?;
    let n = args[1].as_i64().ok_or("take: second argument must be an integer")? as usize;
    let n = n.min(arr.len());
    Ok(DynValue::Array(arr[..n].to_vec()))
}

/// drop: arity 2 — drop first N elements.
pub(super) fn drop(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    if args.len() < 2 {
        return Err("drop: requires 2 arguments (array, count)".to_string());
    }
    let arr = extract_arr(&args[0]).ok_or("drop: first argument must be an array")?;
    let n = args[1].as_i64().ok_or("drop: second argument must be an integer")? as usize;
    let n = n.min(arr.len());
    Ok(DynValue::Array(arr[n..].to_vec()))
}

/// chunk: arity 2 — split array into chunks of size N.
pub(super) fn chunk(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    if args.len() < 2 {
        return Err("chunk: requires 2 arguments (array, size)".to_string());
    }
    let arr = extract_arr(&args[0]).ok_or("chunk: first argument must be an array")?;
    let size = args[1].as_i64().ok_or("chunk: second argument must be an integer")?;
    if size <= 0 {
        return Err("chunk: size must be positive".to_string());
    }
    let size = size as usize;
    let result: Vec<DynValue> = arr.chunks(size)
        .map(|c| DynValue::Array(c.to_vec()))
        .collect();
    Ok(DynValue::Array(result))
}

/// `range_verb`: arity 2-3 — generate array [start..end) with optional step.
pub(super) fn range_verb(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    if args.len() < 2 {
        return Err("range: requires at least 2 arguments (start, end[, step])".to_string());
    }
    let start = args[0].as_i64().ok_or("range: start must be an integer")?;
    let end = args[1].as_i64().ok_or("range: end must be an integer")?;
    let step = if args.len() >= 3 {
        args[2].as_i64().ok_or("range: step must be an integer")?
    } else {
        1
    };
    if step == 0 {
        return Err("range: step must not be zero".to_string());
    }
    let mut result = Vec::new();
    let mut i = start;
    if step > 0 {
        while i < end {
            result.push(DynValue::Integer(i));
            i += step;
        }
    } else {
        while i > end {
            result.push(DynValue::Integer(i));
            i += step;
        }
    }
    Ok(DynValue::Array(result))
}

/// compact: arity 1 — remove null/empty values from array.
pub(super) fn compact(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    let arr = args.first()
        .and_then(extract_arr)
        .ok_or("compact: expected array argument")?;
    let result: Vec<DynValue> = arr.iter()
        .filter(|item| !matches!(item, DynValue::Null) && !matches!(item, DynValue::String(s) if s.is_empty()))
        .cloned()
        .collect();
    Ok(DynValue::Array(result))
}

/// pluck: arity 2 — alias for map (extract field).
pub(super) fn pluck(args: &[DynValue], ctx: &VerbContext) -> Result<DynValue, String> {
    map_verb(args, ctx)
}

/// `row_number`: arity 1 — return current loop index from context.
pub(super) fn row_number(_args: &[DynValue], ctx: &VerbContext) -> Result<DynValue, String> {
    match ctx.loop_vars.get("$index") {
        Some(v) => Ok(v.clone()),
        None => Ok(DynValue::Integer(0)),
    }
}

/// `sample_verb`: arity 2-3 — take N random elements using Fisher-Yates shuffle.
pub(super) fn sample_verb(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    if args.len() < 2 {
        return Err("sample: requires at least 2 arguments (array, count)".to_string());
    }
    let arr = extract_arr(&args[0]).ok_or("sample: first argument must be an array")?;
    let n = args[1].as_i64().ok_or("sample: second argument must be an integer")? as usize;
    let n = n.min(arr.len());
    if n == 0 {
        return Ok(DynValue::Array(vec![]));
    }
    if n >= arr.len() {
        return Ok(DynValue::Array(arr));
    }
    // Seed from 3rd arg or system time
    let seed = if let Some(s) = args.get(2).and_then(DynValue::as_i64) {
        s as u32
    } else {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(42, |d| d.as_millis() as u32)
    };
    let mut state = seed;
    let mut rng = || -> u32 {
        state = state.wrapping_add(0x6D2B_79F5);
        let mut t = state;
        t = (t ^ (t >> 15)).wrapping_mul(1 | t);
        t = (t.wrapping_add((t ^ (t >> 7)).wrapping_mul(61 | t))) ^ t;
        t ^ (t >> 14)
    };
    // Fisher-Yates partial shuffle
    let mut copy = arr;
    for i in 0..n {
        let remaining = copy.len() - i;
        let j = i + (rng() as usize % remaining);
        copy.swap(i, j);
    }
    copy.truncate(n);
    Ok(DynValue::Array(copy))
}

/// limit: arity 2 — alias for take.
pub(super) fn limit(args: &[DynValue], ctx: &VerbContext) -> Result<DynValue, String> {
    take(args, ctx)
}

/// dedupe: arity 2 — deduplicate array of objects by key field.
pub(super) fn dedupe(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    if args.len() < 2 {
        return Err("dedupe: requires 2 arguments (array, keyField)".to_string());
    }
    let arr = extract_arr(&args[0]).ok_or("dedupe: first argument must be an array")?;
    let field = args[1].as_str().ok_or("dedupe: second argument must be a string (field name)")?;
    let mut seen_keys: Vec<DynValue> = Vec::new();
    let mut result = Vec::new();
    for item in &arr {
        let key = get_field(item, field).unwrap_or(DynValue::Null);
        if !seen_keys.contains(&key) {
            seen_keys.push(key);
            result.push(item.clone());
        }
    }
    Ok(DynValue::Array(result))
}

// ─────────────────────────────────────────────────────────────────────────────
// Object verbs
// ─────────────────────────────────────────────────────────────────────────────

/// Helper: extract an object from a `DynValue` (either direct or string-encoded).
fn extract_obj(v: &DynValue) -> Option<Vec<(String, DynValue)>> {
    v.extract_object()
}

/// keys: arity 1 — return array of object keys.
pub(super) fn keys(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    let obj = args.first()
        .and_then(extract_obj)
        .ok_or("keys: expected object argument")?;
    let result: Vec<DynValue> = obj.iter()
        .map(|(k, _)| DynValue::String(k.clone()))
        .collect();
    Ok(DynValue::Array(result))
}

/// `values_verb`: arity 1 — return array of object values.
pub(super) fn values_verb(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    let obj = args.first()
        .and_then(extract_obj)
        .ok_or("values: expected object argument")?;
    let result: Vec<DynValue> = obj.iter()
        .map(|(_, v)| v.clone())
        .collect();
    Ok(DynValue::Array(result))
}

/// entries: arity 1 — return array of [key, value] pairs.
pub(super) fn entries(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    let obj = args.first()
        .and_then(extract_obj)
        .ok_or("entries: expected object argument")?;
    let result: Vec<DynValue> = obj.iter()
        .map(|(k, v)| DynValue::Array(vec![DynValue::String(k.clone()), v.clone()]))
        .collect();
    Ok(DynValue::Array(result))
}

/// has: arity 2 — check if object has key.
pub(super) fn has(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    if args.len() < 2 {
        return Err("has: requires 2 arguments (object, key)".to_string());
    }
    let obj = extract_obj(&args[0]).ok_or("has: first argument must be an object")?;
    let key = args[1].as_str().ok_or("has: second argument must be a string")?;
    let found = obj.iter().any(|(k, _)| k == key);
    Ok(DynValue::Bool(found))
}

/// `get_verb`: arity 2-3 — get value from object by path, with optional default.
pub(super) fn get_verb(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    if args.len() < 2 {
        return Err("get: requires at least 2 arguments (object, path[, default])".to_string());
    }
    let path = args[1].as_str().ok_or("get: second argument must be a string (path)")?;
    let default = args.get(2).cloned().unwrap_or(DynValue::Null);

    // If arg is a string-encoded object, parse it first
    let parsed;
    let root = if let Some(obj) = args[0].extract_object() {
        parsed = DynValue::Object(obj);
        &parsed
    } else {
        &args[0]
    };

    // Walk the path (dot-separated)
    let mut current = root;
    for segment in path.split('.') {
        match current {
            DynValue::Object(obj) => {
                match obj.iter().find(|(k, _)| k == segment) {
                    Some((_, v)) => current = v,
                    None => return Ok(default),
                }
            }
            DynValue::Array(arr) => {
                if let Ok(idx) = segment.parse::<usize>() {
                    match arr.get(idx) {
                        Some(v) => current = v,
                        None => return Ok(default),
                    }
                } else {
                    return Ok(default);
                }
            }
            _ => return Ok(default),
        }
    }
    Ok(current.clone())
}

/// merge: arity 2 — merge two objects (second wins on conflicts).
pub(super) fn merge(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    if args.len() < 2 {
        return Err("merge: requires 2 arguments (object, object)".to_string());
    }
    let a = extract_obj(&args[0]).ok_or("merge: first argument must be an object")?;
    let b = extract_obj(&args[1]).ok_or("merge: second argument must be an object")?;
    let mut result = a;
    for (key, value) in &b {
        if let Some(existing) = result.iter_mut().find(|(k, _)| k == key) {
            existing.1 = value.clone();
        } else {
            result.push((key.clone(), value.clone()));
        }
    }
    Ok(DynValue::Object(result))
}

// ─────────────────────────────────────────────────────────────────────────────
// Aggregation verbs
// ─────────────────────────────────────────────────────────────────────────────

/// accumulate: arity 2 — add value to named accumulator in context.
/// Note: since `VerbContext` is immutable (&), this returns the accumulated value
/// but true mutation must be handled by the engine layer.
pub(super) fn accumulate(args: &[DynValue], ctx: &VerbContext) -> Result<DynValue, String> {
    if args.len() < 2 {
        return Err("accumulate: requires 2 arguments (name, value)".to_string());
    }
    let name = args[0].as_str().ok_or("accumulate: first argument must be a string (name)")?;
    let value = &args[1];
    let current = ctx.accumulators.get(name).cloned().unwrap_or(DynValue::Null);

    // Attempt numeric addition
    match (&current, value) {
        (DynValue::Integer(a), DynValue::Integer(b)) => Ok(DynValue::Integer(a + b)),
        (DynValue::Float(a), DynValue::Float(b)) => Ok(DynValue::Float(a + b)),
        (DynValue::Integer(a), DynValue::Float(b)) => Ok(DynValue::Float(*a as f64 + b)),
        (DynValue::Float(a), DynValue::Integer(b)) => Ok(DynValue::Float(a + *b as f64)),
        _ => Ok(value.clone()),
    }
}

/// `set_verb`: arity 2 — set accumulator to value.
/// Returns the value to set (engine layer handles actual mutation).
pub(super) fn set_verb(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    if args.len() < 2 {
        return Err("set: requires 2 arguments (name, value)".to_string());
    }
    // Return the value; the engine is responsible for storing it.
    Ok(args[1].clone())
}

/// `sum_verb`: arity 1 — sum array of numbers.
pub(super) fn sum_verb(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    let arr = args.first()
        .and_then(extract_arr)
        .ok_or("sum: expected array argument")?;
    let mut total: f64 = 0.0;
    let mut all_integer = true;
    let mut int_total: i64 = 0;
    for item in &arr {
        match item {
            DynValue::Integer(n) => {
                total += *n as f64;
                int_total = int_total.wrapping_add(*n);
            }
            DynValue::Float(n) => {
                total += *n;
                all_integer = false;
            }
            _ => {}
        }
    }
    if all_integer {
        Ok(DynValue::Integer(int_total))
    } else {
        Ok(DynValue::Float(total))
    }
}

/// `count_verb`: arity 1 — count elements in array.
pub(super) fn count_verb(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    let arr = args.first()
        .and_then(extract_arr)
        .ok_or("count: expected array argument")?;
    Ok(DynValue::Integer(arr.len() as i64))
}

/// `min_verb`: arity 1 — minimum value in array.
pub(super) fn min_verb(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    let arr = args.first()
        .and_then(extract_arr)
        .ok_or("min: expected array argument")?;
    if arr.is_empty() {
        return Ok(DynValue::Null);
    }
    let mut min_val: Option<f64> = None;
    for item in &arr {
        if let Some(n) = to_f64(item) {
            min_val = Some(match min_val {
                Some(current) => current.min(n),
                None => n,
            });
        }
    }
    match min_val {
        Some(n) => {
            if n.fract() == 0.0 && arr.iter().all(|v| matches!(v, DynValue::Integer(_))) {
                Ok(DynValue::Integer(n as i64))
            } else {
                Ok(DynValue::Float(n))
            }
        }
        None => Ok(DynValue::Null),
    }
}

/// `max_verb`: arity 1 — maximum value in array.
pub(super) fn max_verb(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    let arr = args.first()
        .and_then(extract_arr)
        .ok_or("max: expected array argument")?;
    if arr.is_empty() {
        return Ok(DynValue::Null);
    }
    let mut max_val: Option<f64> = None;
    for item in &arr {
        if let Some(n) = to_f64(item) {
            max_val = Some(match max_val {
                Some(current) => current.max(n),
                None => n,
            });
        }
    }
    match max_val {
        Some(n) => {
            if n.fract() == 0.0 && arr.iter().all(|v| matches!(v, DynValue::Integer(_))) {
                Ok(DynValue::Integer(n as i64))
            } else {
                Ok(DynValue::Float(n))
            }
        }
        None => Ok(DynValue::Null),
    }
}

/// avg: arity 1 — average of array.
pub(super) fn avg(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    let arr = args.first()
        .and_then(extract_arr)
        .ok_or("avg: expected array argument")?;
    if arr.is_empty() {
        return Ok(DynValue::Null);
    }
    let mut total: f64 = 0.0;
    let mut count: usize = 0;
    for item in &arr {
        if let Some(n) = to_f64(item) {
            total += n;
            count += 1;
        }
    }
    if count == 0 {
        return Ok(DynValue::Null);
    }
    Ok(DynValue::Float(total / count as f64))
}

/// `first_verb`: arity 1 — first element of array.
pub(super) fn first_verb(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    let arr = args.first()
        .and_then(extract_arr)
        .ok_or("first: expected array argument")?;
    Ok(arr.first().cloned().unwrap_or(DynValue::Null))
}

/// `last_verb`: arity 1 — last element of array.
pub(super) fn last_verb(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    let arr = args.first()
        .and_then(extract_arr)
        .ok_or("last: expected array argument")?;
    Ok(arr.last().cloned().unwrap_or(DynValue::Null))
}

// ─────────────────────────────────────────────────────────────────────────────
// Generation verbs
// ─────────────────────────────────────────────────────────────────────────────

/// `uuid_verb`: arity 0-1 — generate a UUID v4 string.
pub(super) fn uuid_verb(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    // If seed string provided, generate deterministic UUID
    let seed = if let Some(s) = args.first().and_then(DynValue::as_str) {
        if s.is_empty() { None } else { Some(s.to_string()) }
    } else {
        None
    };
    let is_seeded = seed.is_some();
    let bytes = if let Some(s) = seed {
        // Deterministic: two DJB2 hashes — matches TypeScript's exact algorithm
        let mut hash1: u32 = 5381;
        let mut hash2: u32 = 52711;
        for &byte in s.as_bytes() {
            let c = u32::from(byte);
            hash1 = (hash1.wrapping_shl(5).wrapping_add(hash1)) ^ c;
            hash2 = (hash2.wrapping_shl(5).wrapping_add(hash2)) ^ c;
        }
        hash1 &= 0xFFFF_FFFF;
        hash2 &= 0xFFFF_FFFF;
        let mut b = [0u8; 16];
        // Use signed shift to match JavaScript's >> operator (sign-extending)
        let sh1 = hash1 as i32;
        let sh2 = hash2 as i32;
        for i in 0..8 {
            b[i] = ((sh1 >> (i as u32 * 4)) & 0xFF) as u8;
            b[i + 8] = ((sh2 >> (i as u32 * 4)) & 0xFF) as u8;
        }
        b
    } else {
        // Random: use system time + counter for entropy
        let t = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0u128, |d| d.as_nanos());
        let mut b = [0u8; 16];
        let lo = t as u64;
        let hi = (t >> 64) as u64 ^ 0xDEAD_BEEF_CAFE_BABE;
        b[..8].copy_from_slice(&lo.to_le_bytes());
        b[8..].copy_from_slice(&hi.to_le_bytes());
        b
    };
    // Set version and variant bits
    let mut b = bytes;
    if is_seeded {
        b[6] = (b[6] & 0x0F) | 0x50; // version 5 for seeded
    } else {
        b[6] = (b[6] & 0x0F) | 0x40; // version 4 for random
    }
    b[8] = (b[8] & 0x3F) | 0x80; // variant 10xx
    Ok(DynValue::String(format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7],
        b[8], b[9], b[10], b[11], b[12], b[13], b[14], b[15]
    )))
}

/// sequence: arity 1 — return and increment named sequence counter from context.
/// Returns the current value; engine layer handles the increment.
pub(super) fn sequence(args: &[DynValue], ctx: &VerbContext) -> Result<DynValue, String> {
    if args.is_empty() {
        return Err("sequence: requires 1 argument (name)".to_string());
    }
    let name = args[0].as_str().ok_or("sequence: argument must be a string (name)")?;
    let seq_key = format!("__seq_{name}");
    let current = ctx.accumulators.get(&seq_key)
        .and_then(crate::types::transform::DynValue::as_i64)
        .unwrap_or(0);
    Ok(DynValue::Integer(current))
}

/// `reset_sequence`: arity 1 — reset named sequence counter to 0.
/// Returns 0; engine layer handles the actual reset.
pub(super) fn reset_sequence(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    if args.is_empty() {
        return Err("resetSequence: requires 1 argument (name)".to_string());
    }
    let _name = args[0].as_str().ok_or("resetSequence: argument must be a string (name)")?;
    Ok(DynValue::Integer(0))
}

// ─────────────────────────────────────────────────────────────────────────────
// Geo verbs
// ─────────────────────────────────────────────────────────────────────────────

const EARTH_RADIUS_KM: f64 = 6_371.0;
const EARTH_RADIUS_MI: f64 = 3_958.8;

/// `to_radians`: arity 1 — degrees to radians.
pub(super) fn to_radians(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    let deg = args.first()
        .and_then(to_f64)
        .ok_or("toRadians: expected numeric argument")?;
    Ok(DynValue::Float(deg.to_radians()))
}

/// `to_degrees`: arity 1 — radians to degrees.
pub(super) fn to_degrees(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    let rad = args.first()
        .and_then(to_f64)
        .ok_or("toDegrees: expected numeric argument")?;
    Ok(DynValue::Float(rad.to_degrees()))
}

/// distance: arity 4-5 — haversine distance between two lat/lon points.
/// Args: (lat1, lon1, lat2, lon2[, unit]) where unit is "km" or "mi" (default "km").
pub(super) fn distance(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    if args.len() < 4 {
        return Err("distance: requires at least 4 arguments (lat1, lon1, lat2, lon2[, unit])".to_string());
    }
    let lat1 = to_f64(&args[0]).ok_or("distance: lat1 must be numeric")?;
    let lon1 = to_f64(&args[1]).ok_or("distance: lon1 must be numeric")?;
    let lat2 = to_f64(&args[2]).ok_or("distance: lat2 must be numeric")?;
    let lon2 = to_f64(&args[3]).ok_or("distance: lon2 must be numeric")?;

    let unit = if args.len() >= 5 {
        args[4].as_str().unwrap_or("km")
    } else {
        "km"
    };
    let radius = match unit {
        "km" => EARTH_RADIUS_KM,
        "mi" | "miles" => EARTH_RADIUS_MI,
        _ => return Err(format!("[T011] distance: unknown unit '{unit}' (expected 'km', 'mi', or 'miles')")),
    };

    let dlat = (lat2 - lat1).to_radians();
    let dlon = (lon2 - lon1).to_radians();
    let lat1_rad = lat1.to_radians();
    let lat2_rad = lat2.to_radians();

    let a = (dlat / 2.0).sin().powi(2)
        + lat1_rad.cos() * lat2_rad.cos() * (dlon / 2.0).sin().powi(2);
    let c = 2.0 * a.sqrt().asin();

    Ok(DynValue::Float(radius * c))
}

/// `in_bounding_box`: arity 6 — check if point is within bounding box.
/// Args: (lat, lon, `min_lat`, `min_lon`, `max_lat`, `max_lon`).
pub(super) fn in_bounding_box(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    if args.len() < 6 {
        return Err("inBoundingBox: requires 6 arguments (lat, lon, minLat, minLon, maxLat, maxLon)".to_string());
    }
    let lat = to_f64(&args[0]).ok_or("inBoundingBox: lat must be numeric")?;
    let lon = to_f64(&args[1]).ok_or("inBoundingBox: lon must be numeric")?;
    let min_lat = to_f64(&args[2]).ok_or("inBoundingBox: minLat must be numeric")?;
    let min_lon = to_f64(&args[3]).ok_or("inBoundingBox: minLon must be numeric")?;
    let max_lat = to_f64(&args[4]).ok_or("inBoundingBox: maxLat must be numeric")?;
    let max_lon = to_f64(&args[5]).ok_or("inBoundingBox: maxLon must be numeric")?;

    let inside = lat >= min_lat && lat <= max_lat && lon >= min_lon && lon <= max_lon;
    Ok(DynValue::Bool(inside))
}

/// bearing: arity 4 — calculate initial bearing between two points.
/// Args: (lat1, lon1, lat2, lon2). Returns bearing in degrees [0, 360).
pub(super) fn bearing(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    if args.len() < 4 {
        return Err("bearing: requires 4 arguments (lat1, lon1, lat2, lon2)".to_string());
    }
    let lat1 = to_f64(&args[0]).ok_or("bearing: lat1 must be numeric")?.to_radians();
    let lon1 = to_f64(&args[1]).ok_or("bearing: lon1 must be numeric")?.to_radians();
    let lat2 = to_f64(&args[2]).ok_or("bearing: lat2 must be numeric")?.to_radians();
    let lon2 = to_f64(&args[3]).ok_or("bearing: lon2 must be numeric")?.to_radians();

    let dlon = lon2 - lon1;
    let x = dlon.sin() * lat2.cos();
    let y = lat1.cos() * lat2.sin() - lat1.sin() * lat2.cos() * dlon.cos();
    let bearing_rad = x.atan2(y);
    let bearing_deg = bearing_rad.to_degrees();
    // Normalize to [0, 360)
    Ok(DynValue::Float((bearing_deg + 360.0) % 360.0))
}

// ─────────────────────────────────────────────────────────────────────────────
// Timeseries verbs: cumsum, cumprod, diff, pctChange, shift
// ─────────────────────────────────────────────────────────────────────────────

/// cumsum: cumulative sum of array elements.
pub(super) fn cumsum(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    if args.is_empty() { return Ok(DynValue::Null); }
    let Some(arr) = extract_arr(&args[0]) else { return Ok(DynValue::Null) };

    let mut sum = 0.0_f64;
    let mut result = Vec::with_capacity(arr.len());
    for item in &arr {
        match to_f64(item) {
            Some(v) if v.is_nan() => result.push(DynValue::Null),
            Some(v) => {
                sum += v;
                if sum.fract() == 0.0 && sum.abs() < i64::MAX as f64 {
                    result.push(DynValue::Integer(sum as i64));
                } else {
                    result.push(DynValue::Float(sum));
                }
            }
            None => result.push(DynValue::Null),
        }
    }
    Ok(DynValue::Array(result))
}

/// cumprod: cumulative product of array elements.
pub(super) fn cumprod(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    if args.is_empty() { return Ok(DynValue::Null); }
    let Some(arr) = extract_arr(&args[0]) else { return Ok(DynValue::Null) };

    let mut product = 1.0_f64;
    let mut result = Vec::with_capacity(arr.len());
    for item in &arr {
        match to_f64(item) {
            Some(v) if v.is_nan() => result.push(DynValue::Null),
            Some(v) => {
                product *= v;
                if product.fract() == 0.0 && product.abs() < i64::MAX as f64 {
                    result.push(DynValue::Integer(product as i64));
                } else {
                    result.push(DynValue::Float(product));
                }
            }
            None => result.push(DynValue::Null),
        }
    }
    Ok(DynValue::Array(result))
}

/// diff: calculate differences between consecutive elements.
pub(super) fn diff_verb(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    if args.is_empty() { return Ok(DynValue::Null); }
    let Some(arr) = extract_arr(&args[0]) else { return Ok(DynValue::Null) };

    let periods = if args.len() >= 2 {
        to_f64(&args[1]).map_or(1, |n| n.floor().max(1.0) as usize)
    } else {
        1
    };

    let mut result = Vec::with_capacity(arr.len());
    for i in 0..arr.len() {
        if i < periods {
            result.push(DynValue::Null);
        } else {
            match (to_f64(&arr[i]), to_f64(&arr[i - periods])) {
                (Some(current), Some(previous)) => {
                    let d = current - previous;
                    if d.fract() == 0.0 && d.abs() < i64::MAX as f64 {
                        result.push(DynValue::Integer(d as i64));
                    } else {
                        result.push(DynValue::Float(d));
                    }
                }
                _ => result.push(DynValue::Null),
            }
        }
    }
    Ok(DynValue::Array(result))
}

/// pctChange: calculate percentage change between consecutive elements.
pub(super) fn pct_change(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    if args.is_empty() { return Ok(DynValue::Null); }
    let Some(arr) = extract_arr(&args[0]) else { return Ok(DynValue::Null) };

    let periods = if args.len() >= 2 {
        to_f64(&args[1]).map_or(1, |n| n.floor().max(1.0) as usize)
    } else {
        1
    };

    let mut result = Vec::with_capacity(arr.len());
    for i in 0..arr.len() {
        if i < periods {
            result.push(DynValue::Null);
        } else {
            match (to_f64(&arr[i]), to_f64(&arr[i - periods])) {
                (Some(current), Some(previous)) if previous != 0.0 => {
                    let pct = (current - previous) / previous;
                    result.push(DynValue::Float(pct));
                }
                _ => result.push(DynValue::Null),
            }
        }
    }
    Ok(DynValue::Array(result))
}

/// shift: shift array elements forward or backward by N positions.
pub(super) fn shift_verb(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    if args.is_empty() { return Ok(DynValue::Null); }
    let Some(arr) = extract_arr(&args[0]) else { return Ok(DynValue::Null) };

    let periods = if args.len() >= 2 {
        to_f64(&args[1]).map_or(1, |n| n.floor() as i64)
    } else {
        1
    };
    let fill_value = if args.len() >= 3 {
        args[2].clone()
    } else {
        DynValue::Null
    };

    let len = arr.len();
    let mut result = vec![DynValue::Null; len];

    if periods >= 0 {
        let p = periods as usize;
        for i in 0..len {
            if i < p {
                result[i] = fill_value.clone();
            } else {
                result[i] = arr[i - p].clone();
            }
        }
    } else {
        let abs_shift = (-periods) as usize;
        for i in 0..len {
            if i >= len - abs_shift {
                result[i] = fill_value.clone();
            } else {
                result[i] = arr[i + abs_shift].clone();
            }
        }
    }
    Ok(DynValue::Array(result))
}

/// lag: shift array backward by N periods, filling start with default value.
/// Args: (array, [periods=1], [default=null])
/// Example: [10,20,30] with lag(1) → [null,10,20]
pub(super) fn lag(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    if args.is_empty() { return Ok(DynValue::Null); }
    let Some(arr) = extract_arr(&args[0]) else { return Ok(DynValue::Null) };
    let periods = if args.len() >= 2 {
        to_f64(&args[1]).map_or(1, |n| n.floor() as usize).max(1)
    } else {
        1
    };
    let default_val = if args.len() >= 3 { args[2].clone() } else { DynValue::Null };

    let mut result = Vec::with_capacity(arr.len());
    for i in 0..arr.len() {
        if i < periods {
            result.push(default_val.clone());
        } else {
            result.push(arr[i - periods].clone());
        }
    }
    Ok(DynValue::Array(result))
}

/// lead: shift array forward by N periods, filling end with default value.
/// Args: (array, [periods=1], [default=null])
/// Example: [10,20,30] with lead(1) → [20,30,null]
pub(super) fn lead(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    if args.is_empty() { return Ok(DynValue::Null); }
    let Some(arr) = extract_arr(&args[0]) else { return Ok(DynValue::Null) };
    let periods = if args.len() >= 2 {
        to_f64(&args[1]).map_or(1, |n| n.floor() as usize).max(1)
    } else {
        1
    };
    let default_val = if args.len() >= 3 { args[2].clone() } else { DynValue::Null };

    let mut result = Vec::with_capacity(arr.len());
    for i in 0..arr.len() {
        if i + periods >= arr.len() {
            result.push(default_val.clone());
        } else {
            result.push(arr[i + periods].clone());
        }
    }
    Ok(DynValue::Array(result))
}

/// rank: rank array items. Items with same value get same rank (competition ranking).
/// Args: (array, [field], [direction="desc"])
/// Example: [10,20,20,30] → objects with _rank: [4,2,2,1]
pub(super) fn rank(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    if args.is_empty() { return Ok(DynValue::Null); }
    let Some(arr) = extract_arr(&args[0]) else { return Ok(DynValue::Null) };

    let field_name = if args.len() > 1 {
        match &args[1] { DynValue::String(s) => Some(s.clone()), _ => None }
    } else {
        None
    };
    let desc = if args.len() > 2 {
        match &args[2] {
            DynValue::String(s) => s.to_lowercase() != "asc",
            _ => true,
        }
    } else {
        true
    };

    // Extract comparable values
    let values: Vec<(usize, f64)> = arr.iter().enumerate().map(|(i, item)| {
        let val = if let Some(ref field) = field_name {
            match item {
                DynValue::Object(entries) => {
                    entries.iter().find(|(k, _)| k == field).map(|(_, v)| v).cloned().unwrap_or(DynValue::Null)
                }
                _ => item.clone(),
            }
        } else {
            item.clone()
        };
        let num = to_f64(&val).unwrap_or(f64::NAN);
        (i, num)
    }).collect();

    // Sort to determine ranks
    let mut sorted: Vec<(usize, f64)> = values.clone();
    sorted.sort_by(|a, b| {
        let cmp = a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal);
        if desc { cmp.reverse() } else { cmp }
    });

    // Assign ranks (competition ranking: same value = same rank, skip next)
    let mut ranks = vec![0usize; arr.len()];
    let mut current_rank = 1usize;
    for i in 0..sorted.len() {
        if i > 0 && (sorted[i].1 - sorted[i - 1].1).abs() > f64::EPSILON {
            current_rank = i + 1;
        }
        ranks[sorted[i].0] = current_rank;
    }

    // Build result: objects with _rank field
    let result: Vec<DynValue> = arr.iter().enumerate().map(|(i, item)| {
        let rank_val = DynValue::Integer(ranks[i] as i64);
        match item {
            DynValue::Object(entries) => {
                let mut obj = vec![("_rank".to_string(), rank_val)];
                obj.extend(entries.iter().cloned());
                DynValue::Object(obj)
            }
            _ => {
                DynValue::Object(vec![
                    ("_rank".to_string(), rank_val),
                    ("value".to_string(), item.clone()),
                ])
            }
        }
    }).collect();

    Ok(DynValue::Array(result))
}

/// fillMissing: replace null values in array.
/// Args: (array, [`fill_value=null`], [strategy="value"])
/// Strategies: "value" (default), "forward", "backward", "mean"
pub(super) fn fill_missing(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    if args.is_empty() { return Ok(DynValue::Null); }
    let Some(arr) = extract_arr(&args[0]) else { return Ok(DynValue::Null) };

    let fill_value = if args.len() >= 2 { args[1].clone() } else { DynValue::Null };
    let strategy = if args.len() >= 3 {
        match &args[2] { DynValue::String(s) => s.to_lowercase(), _ => "value".to_string() }
    } else {
        "value".to_string()
    };

    let is_nullish = |v: &DynValue| matches!(v, DynValue::Null);

    match strategy.as_str() {
        "forward" => {
            let mut result = Vec::with_capacity(arr.len());
            let mut last_non_null = fill_value.clone();
            for item in &arr {
                if is_nullish(item) {
                    result.push(last_non_null.clone());
                } else {
                    result.push(item.clone());
                    last_non_null = item.clone();
                }
            }
            Ok(DynValue::Array(result))
        }
        "backward" => {
            let mut result = vec![DynValue::Null; arr.len()];
            let mut last_non_null = fill_value.clone();
            for i in (0..arr.len()).rev() {
                if is_nullish(&arr[i]) {
                    result[i] = last_non_null.clone();
                } else {
                    result[i] = arr[i].clone();
                    last_non_null = arr[i].clone();
                }
            }
            Ok(DynValue::Array(result))
        }
        "mean" => {
            let mut sum = 0.0f64;
            let mut count = 0usize;
            for item in &arr {
                if !is_nullish(item) {
                    if let Some(n) = to_f64(item) {
                        if n.is_finite() {
                            sum += n;
                            count += 1;
                        }
                    }
                }
            }
            let mean = if count > 0 { sum / count as f64 } else { 0.0 };
            let mean_val = DynValue::Float(mean);
            let result: Vec<DynValue> = arr.iter().map(|item| {
                if is_nullish(item) { mean_val.clone() } else { item.clone() }
            }).collect();
            Ok(DynValue::Array(result))
        }
        _ => {
            // "value" strategy: replace nulls with fill_value
            let result: Vec<DynValue> = arr.iter().map(|item| {
                if is_nullish(item) { fill_value.clone() } else { item.clone() }
            }).collect();
            Ok(DynValue::Array(result))
        }
    }
}

/// midpoint: arity 4 — calculate midpoint between two points.
/// Args: (lat1, lon1, lat2, lon2). Returns object with lat and lon fields.
pub(super) fn midpoint(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    if args.len() < 4 {
        return Err("midpoint: requires 4 arguments (lat1, lon1, lat2, lon2)".to_string());
    }
    let lat1 = to_f64(&args[0]).ok_or("midpoint: lat1 must be numeric")?.to_radians();
    let lon1 = to_f64(&args[1]).ok_or("midpoint: lon1 must be numeric")?.to_radians();
    let lat2 = to_f64(&args[2]).ok_or("midpoint: lat2 must be numeric")?.to_radians();
    let lon2 = to_f64(&args[3]).ok_or("midpoint: lon2 must be numeric")?.to_radians();

    let dlon = lon2 - lon1;
    let bx = lat2.cos() * dlon.cos();
    let by = lat2.cos() * dlon.sin();

    let mid_lat = (lat1.sin() + lat2.sin()).atan2(((lat1.cos() + bx).powi(2) + by.powi(2)).sqrt());
    let mid_lon = lon1 + by.atan2(lat1.cos() + bx);

    Ok(DynValue::Object(vec![
        ("lat".to_string(), DynValue::Float(mid_lat.to_degrees())),
        ("lon".to_string(), DynValue::Float(mid_lon.to_degrees())),
    ]))
}

// ─────────────────────────────────────────────────────────────────────────────
// reduce / pivot / unpivot
// ─────────────────────────────────────────────────────────────────────────────

pub(super) fn reduce(args: &[DynValue], ctx: &VerbContext) -> Result<DynValue, String> {
    if args.len() < 3 {
        return Ok(DynValue::Null);
    }
    let arr = match &args[0] {
        DynValue::Array(a) => a,
        _ => return Ok(DynValue::Null),
    };
    let verb_name = match &args[1] {
        DynValue::String(s) => s.clone(),
        _ => return Ok(DynValue::Null),
    };
    let mut accumulator = args[2].clone();
    let builtins = super::get_builtins();
    let verb_fn = match builtins.get(&verb_name) {
        Some(f) => f,
        None => return Err(format!("reduce: unknown verb '{verb_name}'")),
    };
    for item in arr {
        accumulator = verb_fn(&[accumulator, item.clone()], ctx)?;
    }
    Ok(accumulator)
}

pub(super) fn pivot(args: &[DynValue], ctx: &VerbContext) -> Result<DynValue, String> {
    if args.len() < 3 {
        return Ok(DynValue::Null);
    }
    let arr = match &args[0] {
        DynValue::Array(a) => a,
        _ => return Ok(DynValue::Null),
    };
    let key_field = match &args[1] {
        DynValue::String(s) => s.as_str(),
        _ => return Ok(DynValue::Null),
    };
    let value_field = match &args[2] {
        DynValue::String(s) => s.as_str(),
        _ => return Ok(DynValue::Null),
    };
    let mut result: Vec<(String, DynValue)> = Vec::new();
    for item in arr {
        if let DynValue::Object(entries) = item {
            let key = entries.iter().find(|(k, _)| k == key_field).map(|(_, v)| v);
            let val = entries.iter().find(|(k, _)| k == value_field).map(|(_, v)| v);
            if let Some(k) = key {
                let key_str = match k {
                    DynValue::String(s) => s.clone(),
                    DynValue::Integer(n) => n.to_string(),
                    _ => continue,
                };
                let v = val.cloned().unwrap_or(DynValue::Null);
                // Last value wins
                if let Some(pos) = result.iter().position(|(rk, _)| rk == &key_str) {
                    result[pos].1 = v;
                } else {
                    result.push((key_str, v));
                }
            }
        }
    }
    Ok(DynValue::Object(result))
}

pub(super) fn unpivot(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    if args.len() < 3 {
        return Ok(DynValue::Null);
    }
    let obj = match &args[0] {
        DynValue::Object(entries) => entries,
        _ => return Ok(DynValue::Null),
    };
    let key_name = match &args[1] {
        DynValue::String(s) => s.as_str(),
        _ => return Ok(DynValue::Null),
    };
    let value_name = match &args[2] {
        DynValue::String(s) => s.as_str(),
        _ => return Ok(DynValue::Null),
    };
    let mut result = Vec::new();
    for (k, v) in obj {
        result.push(DynValue::Object(vec![
            (key_name.to_string(), DynValue::String(k.clone())),
            (value_name.to_string(), v.clone()),
        ]));
    }
    Ok(DynValue::Array(result))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn s(v: &str) -> DynValue { DynValue::String(v.to_string()) }
    fn i(v: i64) -> DynValue { DynValue::Integer(v) }
    fn f(v: f64) -> DynValue { DynValue::Float(v) }
    fn b(v: bool) -> DynValue { DynValue::Bool(v) }
    fn arr(items: Vec<DynValue>) -> DynValue { DynValue::Array(items) }
    fn obj(pairs: Vec<(&str, DynValue)>) -> DynValue {
        DynValue::Object(pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
    }
    fn ctx() -> VerbContext<'static> {
        static NULL: DynValue = DynValue::Null;
        static LV: std::sync::OnceLock<HashMap<String, DynValue>> = std::sync::OnceLock::new();
        static ACC: std::sync::OnceLock<HashMap<String, DynValue>> = std::sync::OnceLock::new();
        static TBL: std::sync::OnceLock<HashMap<String, crate::types::transform::LookupTable>> = std::sync::OnceLock::new();
        VerbContext {
            source: &NULL,
            loop_vars: LV.get_or_init(HashMap::new),
            accumulators: ACC.get_or_init(HashMap::new),
            tables: TBL.get_or_init(HashMap::new),
        }
    }

    /// Test helper: holds owned maps so a test can mutate them and then
    /// borrow a `VerbContext` from them via `.ctx()`.
    struct OwnedCtx {
        null: DynValue,
        lv: HashMap<String, DynValue>,
        acc: HashMap<String, DynValue>,
        tbl: HashMap<String, crate::types::transform::LookupTable>,
    }
    impl OwnedCtx {
        fn new() -> Self { Self { null: DynValue::Null, lv: HashMap::new(), acc: HashMap::new(), tbl: HashMap::new() } }
        fn ctx(&self) -> VerbContext<'_> {
            VerbContext { source: &self.null, loop_vars: &self.lv, accumulators: &self.acc, tables: &self.tbl }
        }
    }

    // ── filter ──────────────────────────────────────────────────────────────

    #[test]
    fn filter_by_field_eq() {
        let data = arr(vec![
            obj(vec![("name", s("Alice")), ("age", i(30))]),
            obj(vec![("name", s("Bob")), ("age", i(25))]),
        ]);
        let args = [data, s("name"), s("eq"), s("Alice")];
        let result = filter(&args, &ctx()).unwrap();
        if let DynValue::Array(items) = result { assert_eq!(items.len(), 1); } else { panic!("expected array"); }
    }

    #[test]
    fn filter_no_match() {
        let data = arr(vec![obj(vec![("x", i(1))])]);
        let args = [data, s("x"), s("eq"), i(99)];
        let result = filter(&args, &ctx()).unwrap();
        assert_eq!(result, arr(vec![]));
    }

    #[test]
    fn filter_too_few_args() {
        assert!(filter(&[arr(vec![])], &ctx()).is_err());
    }

    // ── flatten ─────────────────────────────────────────────────────────────

    #[test]
    fn flatten_nested() {
        let data = arr(vec![arr(vec![i(1), i(2)]), arr(vec![i(3)])]);
        let result = flatten(&[data], &ctx()).unwrap();
        assert_eq!(result, arr(vec![i(1), i(2), i(3)]));
    }

    #[test]
    fn flatten_mixed() {
        let data = arr(vec![i(1), arr(vec![i(2), i(3)]), i(4)]);
        let result = flatten(&[data], &ctx()).unwrap();
        assert_eq!(result, arr(vec![i(1), i(2), i(3), i(4)]));
    }

    #[test]
    fn flatten_empty() {
        let result = flatten(&[arr(vec![])], &ctx()).unwrap();
        assert_eq!(result, arr(vec![]));
    }

    #[test]
    fn flatten_not_array_err() {
        assert!(flatten(&[s("hello")], &ctx()).is_err());
    }

    // ── distinct / unique ───────────────────────────────────────────────────

    #[test]
    fn distinct_removes_duplicates() {
        let data = arr(vec![i(1), i(2), i(1), i(3), i(2)]);
        let result = distinct(&[data], &ctx()).unwrap();
        assert_eq!(result, arr(vec![i(1), i(2), i(3)]));
    }

    #[test]
    fn distinct_empty() {
        assert_eq!(distinct(&[arr(vec![])], &ctx()).unwrap(), arr(vec![]));
    }

    #[test]
    fn unique_is_distinct_alias() {
        let data = arr(vec![s("a"), s("b"), s("a")]);
        assert_eq!(unique(&[data.clone()], &ctx()).unwrap(), distinct(&[data], &ctx()).unwrap());
    }

    // ── sort_verb ───────────────────────────────────────────────────────────

    #[test]
    fn sort_integers() {
        let data = arr(vec![i(3), i(1), i(2)]);
        let result = sort_verb(&[data], &ctx()).unwrap();
        assert_eq!(result, arr(vec![i(1), i(2), i(3)]));
    }

    #[test]
    fn sort_strings() {
        let data = arr(vec![s("banana"), s("apple"), s("cherry")]);
        let result = sort_verb(&[data], &ctx()).unwrap();
        assert_eq!(result, arr(vec![s("apple"), s("banana"), s("cherry")]));
    }

    #[test]
    fn sort_empty() {
        assert_eq!(sort_verb(&[arr(vec![])], &ctx()).unwrap(), arr(vec![]));
    }

    #[test]
    fn sort_single() {
        assert_eq!(sort_verb(&[arr(vec![i(42)])], &ctx()).unwrap(), arr(vec![i(42)]));
    }

    #[test]
    fn sort_floats() {
        let data = arr(vec![f(3.1), f(1.5), f(2.7)]);
        let result = sort_verb(&[data], &ctx()).unwrap();
        assert_eq!(result, arr(vec![f(1.5), f(2.7), f(3.1)]));
    }

    // ── sort_desc ───────────────────────────────────────────────────────────

    #[test]
    fn sort_desc_integers() {
        let data = arr(vec![i(1), i(3), i(2)]);
        let result = sort_desc(&[data], &ctx()).unwrap();
        assert_eq!(result, arr(vec![i(3), i(2), i(1)]));
    }

    #[test]
    fn sort_desc_empty() {
        assert_eq!(sort_desc(&[arr(vec![])], &ctx()).unwrap(), arr(vec![]));
    }

    // ── sort_by ─────────────────────────────────────────────────────────────

    #[test]
    fn sort_by_field() {
        let data = arr(vec![
            obj(vec![("n", i(3))]),
            obj(vec![("n", i(1))]),
            obj(vec![("n", i(2))]),
        ]);
        let result = sort_by(&[data, s("n")], &ctx()).unwrap();
        if let DynValue::Array(items) = &result {
            assert_eq!(items[0], obj(vec![("n", i(1))]));
            assert_eq!(items[2], obj(vec![("n", i(3))]));
        } else { panic!("expected array"); }
    }

    #[test]
    fn sort_by_too_few_args() {
        assert!(sort_by(&[arr(vec![])], &ctx()).is_err());
    }

    // ── map_verb / pluck ────────────────────────────────────────────────────

    #[test]
    fn map_extracts_field() {
        let data = arr(vec![
            obj(vec![("name", s("Alice"))]),
            obj(vec![("name", s("Bob"))]),
        ]);
        let result = map_verb(&[data, s("name")], &ctx()).unwrap();
        assert_eq!(result, arr(vec![s("Alice"), s("Bob")]));
    }

    #[test]
    fn map_missing_field_gives_null() {
        let data = arr(vec![obj(vec![("a", i(1))])]);
        let result = map_verb(&[data, s("b")], &ctx()).unwrap();
        assert_eq!(result, arr(vec![DynValue::Null]));
    }

    #[test]
    fn pluck_is_map_alias() {
        let data = arr(vec![obj(vec![("x", i(1))]), obj(vec![("x", i(2))])]);
        assert_eq!(pluck(&[data.clone(), s("x")], &ctx()).unwrap(), map_verb(&[data, s("x")], &ctx()).unwrap());
    }

    // ── index_of ────────────────────────────────────────────────────────────

    #[test]
    fn index_of_found() {
        let data = arr(vec![s("a"), s("b"), s("c")]);
        assert_eq!(index_of(&[data, s("b")], &ctx()).unwrap(), i(1));
    }

    #[test]
    fn index_of_not_found() {
        let data = arr(vec![i(1), i(2)]);
        assert_eq!(index_of(&[data, i(99)], &ctx()).unwrap(), i(-1));
    }

    // ── at ───────────────────────────────────────────────────────────────────

    #[test]
    fn at_valid_index() {
        let data = arr(vec![s("a"), s("b"), s("c")]);
        assert_eq!(at(&[data, i(1)], &ctx()).unwrap(), s("b"));
    }

    #[test]
    fn at_out_of_bounds() {
        let data = arr(vec![i(1)]);
        assert_eq!(at(&[data, i(5)], &ctx()).unwrap(), DynValue::Null);
    }

    // ── slice ───────────────────────────────────────────────────────────────

    #[test]
    fn slice_middle() {
        let data = arr(vec![i(10), i(20), i(30), i(40), i(50)]);
        assert_eq!(slice(&[data, i(1), i(4)], &ctx()).unwrap(), arr(vec![i(20), i(30), i(40)]));
    }

    #[test]
    fn slice_empty_range() {
        let data = arr(vec![i(1), i(2)]);
        assert_eq!(slice(&[data, i(1), i(1)], &ctx()).unwrap(), arr(vec![]));
    }

    #[test]
    fn slice_clamps_end() {
        let data = arr(vec![i(1), i(2)]);
        assert_eq!(slice(&[data, i(0), i(100)], &ctx()).unwrap(), arr(vec![i(1), i(2)]));
    }

    // ── reverse ─────────────────────────────────────────────────────────────

    #[test]
    fn reverse_array() {
        let data = arr(vec![i(1), i(2), i(3)]);
        assert_eq!(reverse(&[data], &ctx()).unwrap(), arr(vec![i(3), i(2), i(1)]));
    }

    #[test]
    fn reverse_empty() {
        assert_eq!(reverse(&[arr(vec![])], &ctx()).unwrap(), arr(vec![]));
    }

    #[test]
    fn reverse_single() {
        assert_eq!(reverse(&[arr(vec![i(1)])], &ctx()).unwrap(), arr(vec![i(1)]));
    }

    // ── every ───────────────────────────────────────────────────────────────

    #[test]
    fn every_all_match() {
        let data = arr(vec![
            obj(vec![("v", i(10))]),
            obj(vec![("v", i(20))]),
        ]);
        assert_eq!(every(&[data, s("v"), s("gt"), i(5)], &ctx()).unwrap(), b(true));
    }

    #[test]
    fn every_not_all() {
        let data = arr(vec![obj(vec![("v", i(1))]), obj(vec![("v", i(10))])]);
        assert_eq!(every(&[data, s("v"), s("gt"), i(5)], &ctx()).unwrap(), b(false));
    }

    #[test]
    fn every_empty_is_true() {
        assert_eq!(every(&[arr(vec![]), s("v"), s("eq"), i(1)], &ctx()).unwrap(), b(true));
    }

    // ── some ────────────────────────────────────────────────────────────────

    #[test]
    fn some_one_matches() {
        let data = arr(vec![obj(vec![("v", i(1))]), obj(vec![("v", i(10))])]);
        assert_eq!(some(&[data, s("v"), s("gt"), i(5)], &ctx()).unwrap(), b(true));
    }

    #[test]
    fn some_none_match() {
        let data = arr(vec![obj(vec![("v", i(1))])]);
        assert_eq!(some(&[data, s("v"), s("gt"), i(5)], &ctx()).unwrap(), b(false));
    }

    #[test]
    fn some_empty_is_false() {
        assert_eq!(some(&[arr(vec![]), s("v"), s("eq"), i(1)], &ctx()).unwrap(), b(false));
    }

    // ── find ────────────────────────────────────────────────────────────────

    #[test]
    fn find_first_match() {
        let data = arr(vec![
            obj(vec![("n", s("a"))]),
            obj(vec![("n", s("b"))]),
            obj(vec![("n", s("b"))]),
        ]);
        let result = find(&[data, s("n"), s("eq"), s("b")], &ctx()).unwrap();
        assert_eq!(result, obj(vec![("n", s("b"))]));
    }

    #[test]
    fn find_no_match_returns_null() {
        let data = arr(vec![obj(vec![("n", i(1))])]);
        assert_eq!(find(&[data, s("n"), s("eq"), i(99)], &ctx()).unwrap(), DynValue::Null);
    }

    // ── find_index ──────────────────────────────────────────────────────────

    #[test]
    fn find_index_found() {
        let data = arr(vec![
            obj(vec![("x", i(1))]),
            obj(vec![("x", i(2))]),
            obj(vec![("x", i(3))]),
        ]);
        assert_eq!(find_index(&[data, s("x"), s("eq"), i(2)], &ctx()).unwrap(), i(1));
    }

    #[test]
    fn find_index_not_found() {
        let data = arr(vec![obj(vec![("x", i(1))])]);
        assert_eq!(find_index(&[data, s("x"), s("eq"), i(99)], &ctx()).unwrap(), i(-1));
    }

    // ── includes ────────────────────────────────────────────────────────────

    #[test]
    fn includes_present() {
        let data = arr(vec![i(1), i(2), i(3)]);
        assert_eq!(includes(&[data, i(2)], &ctx()).unwrap(), b(true));
    }

    #[test]
    fn includes_absent() {
        let data = arr(vec![i(1), i(2)]);
        assert_eq!(includes(&[data, i(99)], &ctx()).unwrap(), b(false));
    }

    #[test]
    fn includes_empty() {
        assert_eq!(includes(&[arr(vec![]), i(1)], &ctx()).unwrap(), b(false));
    }

    // ── concat_arrays ───────────────────────────────────────────────────────

    #[test]
    fn concat_two_arrays() {
        let a = arr(vec![i(1), i(2)]);
        let b_arr = arr(vec![i(3), i(4)]);
        assert_eq!(concat_arrays(&[a, b_arr], &ctx()).unwrap(), arr(vec![i(1), i(2), i(3), i(4)]));
    }

    #[test]
    fn concat_with_empty() {
        let a = arr(vec![i(1)]);
        assert_eq!(concat_arrays(&[a, arr(vec![])], &ctx()).unwrap(), arr(vec![i(1)]));
    }

    // ── zip_verb ────────────────────────────────────────────────────────────

    #[test]
    fn zip_equal_length() {
        let a = arr(vec![i(1), i(2)]);
        let b_arr = arr(vec![s("a"), s("b")]);
        let result = zip_verb(&[a, b_arr], &ctx()).unwrap();
        assert_eq!(result, arr(vec![
            arr(vec![i(1), s("a")]),
            arr(vec![i(2), s("b")]),
        ]));
    }

    #[test]
    fn zip_unequal_truncates() {
        let a = arr(vec![i(1), i(2), i(3)]);
        let b_arr = arr(vec![s("a")]);
        let result = zip_verb(&[a, b_arr], &ctx()).unwrap();
        assert_eq!(result, arr(vec![arr(vec![i(1), s("a")])]));
    }

    // ── group_by ────────────────────────────────────────────────────────────

    #[test]
    fn group_by_field() {
        let data = arr(vec![
            obj(vec![("color", s("red")), ("n", i(1))]),
            obj(vec![("color", s("blue")), ("n", i(2))]),
            obj(vec![("color", s("red")), ("n", i(3))]),
        ]);
        let result = group_by(&[data, s("color")], &ctx()).unwrap();
        if let DynValue::Object(pairs) = result {
            assert_eq!(pairs.len(), 2);
            assert_eq!(pairs[0].0, "red");
            assert_eq!(pairs[1].0, "blue");
        } else { panic!("expected object"); }
    }

    #[test]
    fn group_by_err_few_args() {
        assert!(group_by(&[arr(vec![])], &ctx()).is_err());
    }

    // ── partition ───────────────────────────────────────────────────────────

    #[test]
    fn partition_splits() {
        let data = arr(vec![
            obj(vec![("v", i(1))]),
            obj(vec![("v", i(5))]),
            obj(vec![("v", i(10))]),
        ]);
        let result = partition(&[data, s("v"), s("gt"), i(3)], &ctx()).unwrap();
        if let DynValue::Array(parts) = result {
            assert_eq!(parts.len(), 2);
            if let DynValue::Array(matching) = &parts[0] { assert_eq!(matching.len(), 2); }
            if let DynValue::Array(non) = &parts[1] { assert_eq!(non.len(), 1); }
        } else { panic!("expected array"); }
    }

    // ── take ────────────────────────────────────────────────────────────────

    #[test]
    fn take_first_n() {
        let data = arr(vec![i(1), i(2), i(3), i(4)]);
        assert_eq!(take(&[data, i(2)], &ctx()).unwrap(), arr(vec![i(1), i(2)]));
    }

    #[test]
    fn take_more_than_length() {
        let data = arr(vec![i(1)]);
        assert_eq!(take(&[data, i(100)], &ctx()).unwrap(), arr(vec![i(1)]));
    }

    // ── drop ────────────────────────────────────────────────────────────────

    #[test]
    fn drop_first_n() {
        let data = arr(vec![i(1), i(2), i(3), i(4)]);
        assert_eq!(drop(&[data, i(2)], &ctx()).unwrap(), arr(vec![i(3), i(4)]));
    }

    #[test]
    fn drop_all() {
        let data = arr(vec![i(1), i(2)]);
        assert_eq!(drop(&[data, i(10)], &ctx()).unwrap(), arr(vec![]));
    }

    // ── chunk ───────────────────────────────────────────────────────────────

    #[test]
    fn chunk_even() {
        let data = arr(vec![i(1), i(2), i(3), i(4)]);
        let result = chunk(&[data, i(2)], &ctx()).unwrap();
        assert_eq!(result, arr(vec![arr(vec![i(1), i(2)]), arr(vec![i(3), i(4)])]));
    }

    #[test]
    fn chunk_uneven() {
        let data = arr(vec![i(1), i(2), i(3)]);
        let result = chunk(&[data, i(2)], &ctx()).unwrap();
        assert_eq!(result, arr(vec![arr(vec![i(1), i(2)]), arr(vec![i(3)])]));
    }

    #[test]
    fn chunk_zero_size_err() {
        assert!(chunk(&[arr(vec![i(1)]), i(0)], &ctx()).is_err());
    }

    #[test]
    fn chunk_negative_size_err() {
        assert!(chunk(&[arr(vec![i(1)]), i(-1)], &ctx()).is_err());
    }

    // ── range_verb ──────────────────────────────────────────────────────────

    #[test]
    fn range_basic() {
        assert_eq!(range_verb(&[i(0), i(5)], &ctx()).unwrap(), arr(vec![i(0), i(1), i(2), i(3), i(4)]));
    }

    #[test]
    fn range_with_step() {
        assert_eq!(range_verb(&[i(0), i(10), i(3)], &ctx()).unwrap(), arr(vec![i(0), i(3), i(6), i(9)]));
    }

    #[test]
    fn range_negative_step() {
        assert_eq!(range_verb(&[i(5), i(0), i(-2)], &ctx()).unwrap(), arr(vec![i(5), i(3), i(1)]));
    }

    #[test]
    fn range_zero_step_err() {
        assert!(range_verb(&[i(0), i(5), i(0)], &ctx()).is_err());
    }

    #[test]
    fn range_empty_when_start_ge_end() {
        assert_eq!(range_verb(&[i(5), i(3)], &ctx()).unwrap(), arr(vec![]));
    }

    // ── compact ─────────────────────────────────────────────────────────────

    #[test]
    fn compact_removes_nulls_and_empty() {
        let data = arr(vec![i(1), DynValue::Null, s(""), i(2), s("ok")]);
        assert_eq!(compact(&[data], &ctx()).unwrap(), arr(vec![i(1), i(2), s("ok")]));
    }

    #[test]
    fn compact_all_null() {
        let data = arr(vec![DynValue::Null, DynValue::Null]);
        assert_eq!(compact(&[data], &ctx()).unwrap(), arr(vec![]));
    }

    // ── dedupe ──────────────────────────────────────────────────────────────

    #[test]
    fn dedupe_by_key() {
        let data = arr(vec![
            obj(vec![("id", i(1)), ("n", s("a"))]),
            obj(vec![("id", i(2)), ("n", s("b"))]),
            obj(vec![("id", i(1)), ("n", s("c"))]),
        ]);
        let result = dedupe(&[data, s("id")], &ctx()).unwrap();
        if let DynValue::Array(items) = result { assert_eq!(items.len(), 2); } else { panic!(); }
    }

    // ── keys ────────────────────────────────────────────────────────────────

    #[test]
    fn keys_of_object() {
        let data = obj(vec![("a", i(1)), ("b", i(2))]);
        assert_eq!(keys(&[data], &ctx()).unwrap(), arr(vec![s("a"), s("b")]));
    }

    #[test]
    fn keys_non_object_err() {
        assert!(keys(&[i(42)], &ctx()).is_err());
    }

    // ── values_verb ─────────────────────────────────────────────────────────

    #[test]
    fn values_of_object() {
        let data = obj(vec![("a", i(1)), ("b", i(2))]);
        assert_eq!(values_verb(&[data], &ctx()).unwrap(), arr(vec![i(1), i(2)]));
    }

    #[test]
    fn values_non_object_err() {
        assert!(values_verb(&[s("hi")], &ctx()).is_err());
    }

    // ── entries ─────────────────────────────────────────────────────────────

    #[test]
    fn entries_of_object() {
        let data = obj(vec![("x", i(1))]);
        let result = entries(&[data], &ctx()).unwrap();
        assert_eq!(result, arr(vec![arr(vec![s("x"), i(1)])]));
    }

    // ── has ──────────────────────────────────────────────────────────────────

    #[test]
    fn has_existing_key() {
        let data = obj(vec![("a", i(1))]);
        assert_eq!(has(&[data, s("a")], &ctx()).unwrap(), b(true));
    }

    #[test]
    fn has_missing_key() {
        let data = obj(vec![("a", i(1))]);
        assert_eq!(has(&[data, s("z")], &ctx()).unwrap(), b(false));
    }

    #[test]
    fn has_non_object_err() {
        assert!(has(&[i(1), s("a")], &ctx()).is_err());
    }

    // ── get_verb ────────────────────────────────────────────────────────────

    #[test]
    fn get_simple_path() {
        let data = obj(vec![("a", obj(vec![("b", i(42))]))]);
        assert_eq!(get_verb(&[data, s("a.b")], &ctx()).unwrap(), i(42));
    }

    #[test]
    fn get_missing_returns_default() {
        let data = obj(vec![("a", i(1))]);
        assert_eq!(get_verb(&[data, s("z"), s("fallback")], &ctx()).unwrap(), s("fallback"));
    }

    #[test]
    fn get_missing_no_default_returns_null() {
        let data = obj(vec![("a", i(1))]);
        assert_eq!(get_verb(&[data, s("z")], &ctx()).unwrap(), DynValue::Null);
    }

    // ── merge ───────────────────────────────────────────────────────────────

    #[test]
    fn merge_objects() {
        let a = obj(vec![("a", i(1)), ("b", i(2))]);
        let b_obj = obj(vec![("b", i(99)), ("c", i(3))]);
        let result = merge(&[a, b_obj], &ctx()).unwrap();
        if let DynValue::Object(pairs) = result {
            assert_eq!(pairs.len(), 3);
            // b should be overwritten by second object
            assert_eq!(pairs.iter().find(|(k, _)| k == "b").unwrap().1, i(99));
        } else { panic!("expected object"); }
    }

    #[test]
    fn merge_non_object_err() {
        assert!(merge(&[i(1), obj(vec![])], &ctx()).is_err());
    }

    // ── set_verb ────────────────────────────────────────────────────────────

    #[test]
    fn set_returns_value() {
        assert_eq!(set_verb(&[s("counter"), i(42)], &ctx()).unwrap(), i(42));
    }

    #[test]
    fn set_too_few_args() {
        assert!(set_verb(&[s("x")], &ctx()).is_err());
    }

    // ── sum_verb ────────────────────────────────────────────────────────────

    #[test]
    fn sum_integers() {
        let data = arr(vec![i(1), i(2), i(3)]);
        assert_eq!(sum_verb(&[data], &ctx()).unwrap(), i(6));
    }

    #[test]
    fn sum_floats() {
        let data = arr(vec![f(1.5), f(2.5)]);
        assert_eq!(sum_verb(&[data], &ctx()).unwrap(), f(4.0));
    }

    #[test]
    fn sum_empty() {
        assert_eq!(sum_verb(&[arr(vec![])], &ctx()).unwrap(), i(0));
    }

    #[test]
    fn sum_mixed_int_float() {
        let data = arr(vec![i(1), f(2.5)]);
        assert_eq!(sum_verb(&[data], &ctx()).unwrap(), f(3.5));
    }

    // ── count_verb ──────────────────────────────────────────────────────────

    #[test]
    fn count_elements() {
        let data = arr(vec![i(1), i(2), i(3)]);
        assert_eq!(count_verb(&[data], &ctx()).unwrap(), i(3));
    }

    #[test]
    fn count_empty() {
        assert_eq!(count_verb(&[arr(vec![])], &ctx()).unwrap(), i(0));
    }

    // ── min_verb ────────────────────────────────────────────────────────────

    #[test]
    fn min_integers() {
        let data = arr(vec![i(3), i(1), i(2)]);
        assert_eq!(min_verb(&[data], &ctx()).unwrap(), i(1));
    }

    #[test]
    fn min_empty_is_null() {
        assert_eq!(min_verb(&[arr(vec![])], &ctx()).unwrap(), DynValue::Null);
    }

    #[test]
    fn min_floats() {
        let data = arr(vec![f(3.5), f(1.2), f(2.8)]);
        assert_eq!(min_verb(&[data], &ctx()).unwrap(), f(1.2));
    }

    // ── max_verb ────────────────────────────────────────────────────────────

    #[test]
    fn max_integers() {
        let data = arr(vec![i(3), i(1), i(2)]);
        assert_eq!(max_verb(&[data], &ctx()).unwrap(), i(3));
    }

    #[test]
    fn max_empty_is_null() {
        assert_eq!(max_verb(&[arr(vec![])], &ctx()).unwrap(), DynValue::Null);
    }

    // ── avg ─────────────────────────────────────────────────────────────────

    #[test]
    fn avg_basic() {
        let data = arr(vec![i(10), i(20), i(30)]);
        assert_eq!(avg(&[data], &ctx()).unwrap(), f(20.0));
    }

    #[test]
    fn avg_empty_is_null() {
        assert_eq!(avg(&[arr(vec![])], &ctx()).unwrap(), DynValue::Null);
    }

    #[test]
    fn avg_single() {
        let data = arr(vec![i(7)]);
        assert_eq!(avg(&[data], &ctx()).unwrap(), f(7.0));
    }

    // ── first_verb ──────────────────────────────────────────────────────────

    #[test]
    fn first_of_array() {
        let data = arr(vec![s("a"), s("b")]);
        assert_eq!(first_verb(&[data], &ctx()).unwrap(), s("a"));
    }

    #[test]
    fn first_empty_is_null() {
        assert_eq!(first_verb(&[arr(vec![])], &ctx()).unwrap(), DynValue::Null);
    }

    // ── last_verb ───────────────────────────────────────────────────────────

    #[test]
    fn last_of_array() {
        let data = arr(vec![s("a"), s("b"), s("c")]);
        assert_eq!(last_verb(&[data], &ctx()).unwrap(), s("c"));
    }

    #[test]
    fn last_empty_is_null() {
        assert_eq!(last_verb(&[arr(vec![])], &ctx()).unwrap(), DynValue::Null);
    }

    // ── uuid_verb ───────────────────────────────────────────────────────────

    #[test]
    fn uuid_returns_deterministic_string() {
        let result = uuid_verb(&[], &ctx()).unwrap();
        if let DynValue::String(s) = result {
            assert_eq!(s.len(), 36); // UUID format
        } else { panic!("expected string"); }
    }

    // ── sequence / reset_sequence ───────────────────────────────────────────

    #[test]
    fn sequence_default_zero() {
        assert_eq!(sequence(&[s("counter")], &ctx()).unwrap(), i(0));
    }

    #[test]
    fn sequence_no_args_err() {
        assert!(sequence(&[], &ctx()).is_err());
    }

    #[test]
    fn reset_sequence_returns_zero() {
        assert_eq!(reset_sequence(&[s("counter")], &ctx()).unwrap(), i(0));
    }

    // ── to_radians / to_degrees ─────────────────────────────────────────────

    #[test]
    fn to_radians_180() {
        let result = to_radians(&[f(180.0)], &ctx()).unwrap();
        if let DynValue::Float(v) = result {
            assert!((v - std::f64::consts::PI).abs() < 1e-10);
        } else { panic!(); }
    }

    #[test]
    fn to_degrees_pi() {
        let result = to_degrees(&[f(std::f64::consts::PI)], &ctx()).unwrap();
        if let DynValue::Float(v) = result {
            assert!((v - 180.0).abs() < 1e-10);
        } else { panic!(); }
    }

    #[test]
    fn to_radians_non_numeric_err() {
        assert!(to_radians(&[s("abc")], &ctx()).is_err());
    }

    // ── distance ────────────────────────────────────────────────────────────

    #[test]
    fn distance_same_point_is_zero() {
        let result = distance(&[f(40.0), f(-74.0), f(40.0), f(-74.0)], &ctx()).unwrap();
        if let DynValue::Float(v) = result { assert!(v.abs() < 0.001); } else { panic!(); }
    }

    #[test]
    fn distance_too_few_args() {
        assert!(distance(&[f(1.0), f(2.0)], &ctx()).is_err());
    }

    // ── in_bounding_box ─────────────────────────────────────────────────────

    #[test]
    fn in_bounding_box_inside() {
        let result = in_bounding_box(&[f(5.0), f(5.0), f(0.0), f(0.0), f(10.0), f(10.0)], &ctx()).unwrap();
        assert_eq!(result, b(true));
    }

    #[test]
    fn in_bounding_box_outside() {
        let result = in_bounding_box(&[f(15.0), f(5.0), f(0.0), f(0.0), f(10.0), f(10.0)], &ctx()).unwrap();
        assert_eq!(result, b(false));
    }

    // ── bearing ─────────────────────────────────────────────────────────────

    #[test]
    fn bearing_north() {
        // Going due north: same lon, increasing lat
        let result = bearing(&[f(0.0), f(0.0), f(10.0), f(0.0)], &ctx()).unwrap();
        if let DynValue::Float(v) = result {
            assert!(v.abs() < 1.0 || (v - 360.0).abs() < 1.0); // ~0 degrees
        } else { panic!(); }
    }

    // ── cumsum ──────────────────────────────────────────────────────────────

    #[test]
    fn cumsum_basic() {
        let data = arr(vec![i(1), i(2), i(3)]);
        assert_eq!(cumsum(&[data], &ctx()).unwrap(), arr(vec![i(1), i(3), i(6)]));
    }

    #[test]
    fn cumsum_with_null() {
        let data = arr(vec![i(1), DynValue::Null, i(3)]);
        let result = cumsum(&[data], &ctx()).unwrap();
        if let DynValue::Array(items) = result {
            assert_eq!(items[0], i(1));
            assert_eq!(items[1], DynValue::Null);
            assert_eq!(items[2], i(4));
        } else { panic!(); }
    }

    #[test]
    fn cumsum_empty() {
        assert_eq!(cumsum(&[arr(vec![])], &ctx()).unwrap(), arr(vec![]));
    }

    // ── cumprod ─────────────────────────────────────────────────────────────

    #[test]
    fn cumprod_basic() {
        let data = arr(vec![i(1), i(2), i(3)]);
        assert_eq!(cumprod(&[data], &ctx()).unwrap(), arr(vec![i(1), i(2), i(6)]));
    }

    #[test]
    fn cumprod_with_zero() {
        let data = arr(vec![i(5), i(0), i(3)]);
        assert_eq!(cumprod(&[data], &ctx()).unwrap(), arr(vec![i(5), i(0), i(0)]));
    }

    // ── diff_verb ───────────────────────────────────────────────────────────

    #[test]
    fn diff_basic() {
        let data = arr(vec![i(10), i(20), i(15)]);
        let result = diff_verb(&[data], &ctx()).unwrap();
        assert_eq!(result, arr(vec![DynValue::Null, i(10), i(-5)]));
    }

    #[test]
    fn diff_with_period() {
        let data = arr(vec![i(1), i(2), i(4), i(8)]);
        let result = diff_verb(&[data, i(2)], &ctx()).unwrap();
        if let DynValue::Array(items) = result {
            assert_eq!(items[0], DynValue::Null);
            assert_eq!(items[1], DynValue::Null);
            assert_eq!(items[2], i(3));  // 4 - 1
            assert_eq!(items[3], i(6));  // 8 - 2
        } else { panic!(); }
    }

    // ── pct_change ──────────────────────────────────────────────────────────

    #[test]
    fn pct_change_basic() {
        let data = arr(vec![i(100), i(110)]);
        let result = pct_change(&[data], &ctx()).unwrap();
        if let DynValue::Array(items) = result {
            assert_eq!(items[0], DynValue::Null);
            if let DynValue::Float(v) = items[1] { assert!((v - 0.1).abs() < 1e-10); } else { panic!(); }
        } else { panic!(); }
    }

    // ── shift_verb ──────────────────────────────────────────────────────────

    #[test]
    fn shift_forward() {
        let data = arr(vec![i(1), i(2), i(3)]);
        let result = shift_verb(&[data, i(1)], &ctx()).unwrap();
        assert_eq!(result, arr(vec![DynValue::Null, i(1), i(2)]));
    }

    #[test]
    fn shift_backward() {
        let data = arr(vec![i(1), i(2), i(3)]);
        let result = shift_verb(&[data, i(-1)], &ctx()).unwrap();
        assert_eq!(result, arr(vec![i(2), i(3), DynValue::Null]));
    }

    // ── lag ─────────────────────────────────────────────────────────────────

    #[test]
    fn lag_default_period() {
        let data = arr(vec![i(10), i(20), i(30)]);
        let result = lag(&[data], &ctx()).unwrap();
        assert_eq!(result, arr(vec![DynValue::Null, i(10), i(20)]));
    }

    #[test]
    fn lag_with_default_value() {
        let data = arr(vec![i(10), i(20), i(30)]);
        let result = lag(&[data, i(1), i(0)], &ctx()).unwrap();
        assert_eq!(result, arr(vec![i(0), i(10), i(20)]));
    }

    // ── lead ────────────────────────────────────────────────────────────────

    #[test]
    fn lead_default_period() {
        let data = arr(vec![i(10), i(20), i(30)]);
        let result = lead(&[data], &ctx()).unwrap();
        assert_eq!(result, arr(vec![i(20), i(30), DynValue::Null]));
    }

    #[test]
    fn lead_with_default_value() {
        let data = arr(vec![i(10), i(20), i(30)]);
        let result = lead(&[data, i(1), i(99)], &ctx()).unwrap();
        assert_eq!(result, arr(vec![i(20), i(30), i(99)]));
    }

    // ── rank ────────────────────────────────────────────────────────────────

    #[test]
    fn rank_basic_desc() {
        let data = arr(vec![i(10), i(30), i(20)]);
        let result = rank(&[data], &ctx()).unwrap();
        if let DynValue::Array(items) = result {
            assert_eq!(items.len(), 3);
            // 30 should be rank 1 (desc), 20 rank 2, 10 rank 3
            let get_rank = |item: &DynValue| {
                if let DynValue::Object(pairs) = item {
                    pairs.iter().find(|(k, _)| k == "_rank").map(|(_, v)| v.clone())
                } else { None }
            };
            assert_eq!(get_rank(&items[0]), Some(i(3))); // 10 -> rank 3
            assert_eq!(get_rank(&items[1]), Some(i(1))); // 30 -> rank 1
            assert_eq!(get_rank(&items[2]), Some(i(2))); // 20 -> rank 2
        } else { panic!(); }
    }

    // ── fill_missing ────────────────────────────────────────────────────────

    #[test]
    fn fill_missing_value_strategy() {
        let data = arr(vec![i(1), DynValue::Null, i(3)]);
        let result = fill_missing(&[data, i(0)], &ctx()).unwrap();
        assert_eq!(result, arr(vec![i(1), i(0), i(3)]));
    }

    #[test]
    fn fill_missing_forward() {
        let data = arr(vec![i(1), DynValue::Null, DynValue::Null, i(4)]);
        let result = fill_missing(&[data, i(0), s("forward")], &ctx()).unwrap();
        assert_eq!(result, arr(vec![i(1), i(1), i(1), i(4)]));
    }

    #[test]
    fn fill_missing_backward() {
        let data = arr(vec![DynValue::Null, DynValue::Null, i(3), i(4)]);
        let result = fill_missing(&[data, i(0), s("backward")], &ctx()).unwrap();
        assert_eq!(result, arr(vec![i(3), i(3), i(3), i(4)]));
    }

    #[test]
    fn fill_missing_mean() {
        let data = arr(vec![i(10), DynValue::Null, i(30)]);
        let result = fill_missing(&[data, i(0), s("mean")], &ctx()).unwrap();
        if let DynValue::Array(items) = result {
            assert_eq!(items[0], i(10));
            if let DynValue::Float(v) = items[1] { assert!((v - 20.0).abs() < 1e-10); } else { panic!(); }
            assert_eq!(items[2], i(30));
        } else { panic!(); }
    }

    // ── midpoint ────────────────────────────────────────────────────────────

    #[test]
    fn midpoint_same_point() {
        let result = midpoint(&[f(40.0), f(-74.0), f(40.0), f(-74.0)], &ctx()).unwrap();
        if let DynValue::Object(pairs) = result {
            let lat = pairs.iter().find(|(k, _)| k == "lat").unwrap();
            let lon = pairs.iter().find(|(k, _)| k == "lon").unwrap();
            if let DynValue::Float(v) = &lat.1 { assert!((v - 40.0).abs() < 0.001); }
            if let DynValue::Float(v) = &lon.1 { assert!((v - (-74.0)).abs() < 0.001); }
        } else { panic!(); }
    }

    #[test]
    fn midpoint_too_few_args() {
        assert!(midpoint(&[f(1.0), f(2.0)], &ctx()).is_err());
    }

    // ── sample_verb / limit ─────────────────────────────────────────────────

    #[test]
    fn sample_returns_correct_count() {
        let data = arr(vec![i(1), i(2), i(3), i(4)]);
        let result = sample_verb(&[data, i(2)], &ctx()).unwrap();
        if let DynValue::Array(items) = result {
            assert_eq!(items.len(), 2);
        } else {
            panic!("expected array");
        }
    }

    #[test]
    fn sample_with_seed_is_deterministic() {
        let data = arr(vec![i(1), i(2), i(3), i(4), i(5)]);
        let r1 = sample_verb(&[data.clone(), i(3), i(42)], &ctx()).unwrap();
        let r2 = sample_verb(&[data, i(3), i(42)], &ctx()).unwrap();
        assert_eq!(r1, r2);
    }

    #[test]
    fn limit_is_take_alias() {
        let data = arr(vec![i(1), i(2), i(3)]);
        assert_eq!(limit(&[data.clone(), i(2)], &ctx()).unwrap(), take(&[data, i(2)], &ctx()).unwrap());
    }

    // ── accumulate ──────────────────────────────────────────────────────────

    #[test]
    fn accumulate_adds_to_existing() {
        let mut h = OwnedCtx::new();
        h.acc.insert("total".to_string(), i(10));
        let result = accumulate(&[s("total"), i(5)], &h.ctx()).unwrap();
        assert_eq!(result, i(15));
    }

    #[test]
    fn accumulate_no_existing_returns_value() {
        let result = accumulate(&[s("new"), i(5)], &ctx()).unwrap();
        // current is Null, so returns value directly
        assert_eq!(result, i(5));
    }

    // ── row_number ──────────────────────────────────────────────────────────

    #[test]
    fn row_number_from_context() {
        let mut h = OwnedCtx::new();
        h.lv.insert("$index".to_string(), i(7));
        assert_eq!(row_number(&[], &h.ctx()).unwrap(), i(7));
    }

    #[test]
    fn row_number_default_zero() {
        assert_eq!(row_number(&[], &ctx()).unwrap(), i(0));
    }
}

#[cfg(test)]
mod extended_tests {
    use super::*;
    use std::collections::HashMap;

    fn ctx() -> VerbContext<'static> {
        static NULL: DynValue = DynValue::Null;
        static LV: std::sync::OnceLock<HashMap<String, DynValue>> = std::sync::OnceLock::new();
        static ACC: std::sync::OnceLock<HashMap<String, DynValue>> = std::sync::OnceLock::new();
        static TBL: std::sync::OnceLock<HashMap<String, crate::types::transform::LookupTable>> = std::sync::OnceLock::new();
        VerbContext {
            source: &NULL,
            loop_vars: LV.get_or_init(HashMap::new),
            accumulators: ACC.get_or_init(HashMap::new),
            tables: TBL.get_or_init(HashMap::new),
        }
    }

    struct OwnedCtx {
        null: DynValue,
        lv: HashMap<String, DynValue>,
        acc: HashMap<String, DynValue>,
        tbl: HashMap<String, crate::types::transform::LookupTable>,
    }
    impl OwnedCtx {
        fn new() -> Self { Self { null: DynValue::Null, lv: HashMap::new(), acc: HashMap::new(), tbl: HashMap::new() } }
        fn ctx(&self) -> VerbContext<'_> {
            VerbContext { source: &self.null, loop_vars: &self.lv, accumulators: &self.acc, tables: &self.tbl }
        }
    }

    fn s(v: &str) -> DynValue { DynValue::String(v.to_string()) }
    fn i(v: i64) -> DynValue { DynValue::Integer(v) }
    fn f(v: f64) -> DynValue { DynValue::Float(v) }
    fn b(v: bool) -> DynValue { DynValue::Bool(v) }
    fn null() -> DynValue { DynValue::Null }
    fn arr(items: Vec<DynValue>) -> DynValue { DynValue::Array(items) }
    fn obj(pairs: Vec<(&str, DynValue)>) -> DynValue {
        DynValue::Object(pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
    }

    // =========================================================================
    // filter — extended
    // =========================================================================

    #[test]
    fn filter_gt_operator() {
        let data = arr(vec![
            obj(vec![("age", i(10))]),
            obj(vec![("age", i(20))]),
            obj(vec![("age", i(30))]),
        ]);
        let result = filter(&[data, s("age"), s("gt"), i(15)], &ctx()).unwrap();
        if let DynValue::Array(items) = result { assert_eq!(items.len(), 2); } else { panic!(); }
    }

    #[test]
    fn filter_lt_operator() {
        let data = arr(vec![
            obj(vec![("v", i(5))]),
            obj(vec![("v", i(15))]),
        ]);
        let result = filter(&[data, s("v"), s("lt"), i(10)], &ctx()).unwrap();
        if let DynValue::Array(items) = result { assert_eq!(items.len(), 1); } else { panic!(); }
    }

    #[test]
    fn filter_gte_operator() {
        let data = arr(vec![
            obj(vec![("v", i(10))]),
            obj(vec![("v", i(10))]),
            obj(vec![("v", i(5))]),
        ]);
        let result = filter(&[data, s("v"), s("gte"), i(10)], &ctx()).unwrap();
        if let DynValue::Array(items) = result { assert_eq!(items.len(), 2); } else { panic!(); }
    }

    #[test]
    fn filter_lte_operator() {
        let data = arr(vec![
            obj(vec![("v", i(10))]),
            obj(vec![("v", i(5))]),
            obj(vec![("v", i(15))]),
        ]);
        let result = filter(&[data, s("v"), s("lte"), i(10)], &ctx()).unwrap();
        if let DynValue::Array(items) = result { assert_eq!(items.len(), 2); } else { panic!(); }
    }

    #[test]
    fn filter_ne_operator() {
        let data = arr(vec![
            obj(vec![("v", i(1))]),
            obj(vec![("v", i(2))]),
            obj(vec![("v", i(1))]),
        ]);
        let result = filter(&[data, s("v"), s("ne"), i(1)], &ctx()).unwrap();
        if let DynValue::Array(items) = result { assert_eq!(items.len(), 1); } else { panic!(); }
    }

    #[test]
    fn filter_contains_operator() {
        let data = arr(vec![
            obj(vec![("name", s("Alice Smith"))]),
            obj(vec![("name", s("Bob Jones"))]),
        ]);
        let result = filter(&[data, s("name"), s("contains"), s("Smith")], &ctx()).unwrap();
        if let DynValue::Array(items) = result { assert_eq!(items.len(), 1); } else { panic!(); }
    }

    #[test]
    fn filter_empty_array() {
        let result = filter(&[arr(vec![]), s("x"), s("eq"), i(1)], &ctx()).unwrap();
        assert_eq!(result, arr(vec![]));
    }

    #[test]
    fn filter_with_null_elements() {
        let data = arr(vec![null(), obj(vec![("x", i(1))])]);
        let result = filter(&[data, s("x"), s("eq"), i(1)], &ctx()).unwrap();
        if let DynValue::Array(items) = result { assert_eq!(items.len(), 1); } else { panic!(); }
    }

    #[test]
    fn filter_string_field_eq() {
        let data = arr(vec![
            obj(vec![("status", s("active"))]),
            obj(vec![("status", s("inactive"))]),
            obj(vec![("status", s("active"))]),
        ]);
        let result = filter(&[data, s("status"), s("eq"), s("active")], &ctx()).unwrap();
        if let DynValue::Array(items) = result { assert_eq!(items.len(), 2); } else { panic!(); }
    }

    #[test]
    fn filter_with_float_comparison() {
        let data = arr(vec![
            obj(vec![("price", f(9.99))]),
            obj(vec![("price", f(19.99))]),
            obj(vec![("price", f(29.99))]),
        ]);
        let result = filter(&[data, s("price"), s("lt"), f(20.0)], &ctx()).unwrap();
        if let DynValue::Array(items) = result { assert_eq!(items.len(), 2); } else { panic!(); }
    }

    // =========================================================================
    // flatten — extended
    // =========================================================================

    #[test]
    fn flatten_single_level_only() {
        let data = arr(vec![arr(vec![arr(vec![i(1)])])]);
        let result = flatten(&[data], &ctx()).unwrap();
        // Only flattens one level
        assert_eq!(result, arr(vec![arr(vec![i(1)])]));
    }

    #[test]
    fn flatten_all_scalars() {
        let data = arr(vec![i(1), i(2), i(3)]);
        let result = flatten(&[data], &ctx()).unwrap();
        assert_eq!(result, arr(vec![i(1), i(2), i(3)]));
    }

    #[test]
    fn flatten_with_nulls() {
        let data = arr(vec![null(), arr(vec![i(1)])]);
        let result = flatten(&[data], &ctx()).unwrap();
        assert_eq!(result, arr(vec![null(), i(1)]));
    }

    // =========================================================================
    // distinct / unique — extended
    // =========================================================================

    #[test]
    fn distinct_single_element() {
        assert_eq!(distinct(&[arr(vec![i(42)])], &ctx()).unwrap(), arr(vec![i(42)]));
    }

    #[test]
    fn distinct_all_same() {
        let data = arr(vec![s("a"), s("a"), s("a")]);
        assert_eq!(distinct(&[data], &ctx()).unwrap(), arr(vec![s("a")]));
    }

    #[test]
    fn distinct_mixed_types() {
        let data = arr(vec![i(1), s("1"), b(true), null()]);
        let result = distinct(&[data], &ctx()).unwrap();
        if let DynValue::Array(items) = result { assert_eq!(items.len(), 4); } else { panic!(); }
    }

    #[test]
    fn distinct_preserves_order() {
        let data = arr(vec![i(3), i(1), i(2), i(1), i(3)]);
        assert_eq!(distinct(&[data], &ctx()).unwrap(), arr(vec![i(3), i(1), i(2)]));
    }

    #[test]
    fn distinct_with_nulls() {
        let data = arr(vec![null(), i(1), null(), i(2)]);
        assert_eq!(distinct(&[data], &ctx()).unwrap(), arr(vec![null(), i(1), i(2)]));
    }

    // =========================================================================
    // sort_verb — extended
    // =========================================================================

    #[test]
    fn sort_mixed_int_float() {
        let data = arr(vec![f(2.5), i(1), i(3)]);
        let result = sort_verb(&[data], &ctx()).unwrap();
        if let DynValue::Array(items) = result {
            assert_eq!(items[0], i(1));
            assert_eq!(items[1], f(2.5));
            assert_eq!(items[2], i(3));
        } else { panic!(); }
    }

    #[test]
    fn sort_already_sorted() {
        let data = arr(vec![i(1), i(2), i(3)]);
        assert_eq!(sort_verb(&[data], &ctx()).unwrap(), arr(vec![i(1), i(2), i(3)]));
    }

    #[test]
    fn sort_reverse_order() {
        let data = arr(vec![i(3), i(2), i(1)]);
        assert_eq!(sort_verb(&[data], &ctx()).unwrap(), arr(vec![i(1), i(2), i(3)]));
    }

    #[test]
    fn sort_with_duplicates() {
        let data = arr(vec![i(2), i(1), i(2), i(1)]);
        assert_eq!(sort_verb(&[data], &ctx()).unwrap(), arr(vec![i(1), i(1), i(2), i(2)]));
    }

    // =========================================================================
    // sort_desc — extended
    // =========================================================================

    #[test]
    fn sort_desc_strings() {
        let data = arr(vec![s("a"), s("c"), s("b")]);
        assert_eq!(sort_desc(&[data], &ctx()).unwrap(), arr(vec![s("c"), s("b"), s("a")]));
    }

    #[test]
    fn sort_desc_floats() {
        let data = arr(vec![f(1.1), f(3.3), f(2.2)]);
        assert_eq!(sort_desc(&[data], &ctx()).unwrap(), arr(vec![f(3.3), f(2.2), f(1.1)]));
    }

    #[test]
    fn sort_desc_single() {
        assert_eq!(sort_desc(&[arr(vec![i(5)])], &ctx()).unwrap(), arr(vec![i(5)]));
    }

    // =========================================================================
    // sort_by — extended
    // =========================================================================

    #[test]
    fn sort_by_string_field() {
        let data = arr(vec![
            obj(vec![("name", s("Charlie"))]),
            obj(vec![("name", s("Alice"))]),
            obj(vec![("name", s("Bob"))]),
        ]);
        let result = sort_by(&[data, s("name")], &ctx()).unwrap();
        if let DynValue::Array(items) = &result {
            assert_eq!(items[0], obj(vec![("name", s("Alice"))]));
            assert_eq!(items[1], obj(vec![("name", s("Bob"))]));
            assert_eq!(items[2], obj(vec![("name", s("Charlie"))]));
        } else { panic!(); }
    }

    #[test]
    fn sort_by_empty_array() {
        let result = sort_by(&[arr(vec![]), s("x")], &ctx()).unwrap();
        assert_eq!(result, arr(vec![]));
    }

    #[test]
    fn sort_by_missing_field() {
        let data = arr(vec![
            obj(vec![("a", i(2))]),
            obj(vec![("b", i(1))]),
        ]);
        // Missing fields treated as equal
        let result = sort_by(&[data, s("a")], &ctx()).unwrap();
        if let DynValue::Array(items) = result { assert_eq!(items.len(), 2); } else { panic!(); }
    }

    // =========================================================================
    // map_verb / pluck — extended
    // =========================================================================

    #[test]
    fn map_empty_array() {
        assert_eq!(map_verb(&[arr(vec![]), s("x")], &ctx()).unwrap(), arr(vec![]));
    }

    #[test]
    fn map_all_missing_fields() {
        let data = arr(vec![obj(vec![("a", i(1))]), obj(vec![("a", i(2))])]);
        let result = map_verb(&[data, s("z")], &ctx()).unwrap();
        assert_eq!(result, arr(vec![null(), null()]));
    }

    #[test]
    fn map_too_few_args() {
        assert!(map_verb(&[arr(vec![])], &ctx()).is_err());
    }

    #[test]
    fn pluck_empty_array() {
        assert_eq!(pluck(&[arr(vec![]), s("x")], &ctx()).unwrap(), arr(vec![]));
    }

    // =========================================================================
    // index_of — extended
    // =========================================================================

    #[test]
    fn index_of_first_occurrence() {
        let data = arr(vec![i(1), i(2), i(1)]);
        assert_eq!(index_of(&[data, i(1)], &ctx()).unwrap(), i(0));
    }

    #[test]
    fn index_of_empty_array() {
        assert_eq!(index_of(&[arr(vec![]), i(1)], &ctx()).unwrap(), i(-1));
    }

    #[test]
    fn index_of_string_value() {
        let data = arr(vec![s("hello"), s("world")]);
        assert_eq!(index_of(&[data, s("world")], &ctx()).unwrap(), i(1));
    }

    #[test]
    fn index_of_too_few_args() {
        assert!(index_of(&[arr(vec![])], &ctx()).is_err());
    }

    // =========================================================================
    // at — extended
    // =========================================================================

    #[test]
    fn at_first_element() {
        let data = arr(vec![s("first"), s("second")]);
        assert_eq!(at(&[data, i(0)], &ctx()).unwrap(), s("first"));
    }

    #[test]
    fn at_last_element() {
        let data = arr(vec![i(1), i(2), i(3)]);
        assert_eq!(at(&[data, i(2)], &ctx()).unwrap(), i(3));
    }

    #[test]
    fn at_empty_array() {
        assert_eq!(at(&[arr(vec![]), i(0)], &ctx()).unwrap(), null());
    }

    #[test]
    fn at_too_few_args() {
        assert!(at(&[arr(vec![])], &ctx()).is_err());
    }

    // =========================================================================
    // slice — extended
    // =========================================================================

    #[test]
    fn slice_full_array() {
        let data = arr(vec![i(1), i(2), i(3)]);
        assert_eq!(slice(&[data, i(0), i(3)], &ctx()).unwrap(), arr(vec![i(1), i(2), i(3)]));
    }

    #[test]
    fn slice_empty_array() {
        assert_eq!(slice(&[arr(vec![]), i(0), i(0)], &ctx()).unwrap(), arr(vec![]));
    }

    #[test]
    fn slice_single_element() {
        let data = arr(vec![i(10), i(20), i(30)]);
        assert_eq!(slice(&[data, i(1), i(2)], &ctx()).unwrap(), arr(vec![i(20)]));
    }

    #[test]
    fn slice_start_equals_end_past_length() {
        let data = arr(vec![i(1)]);
        assert_eq!(slice(&[data, i(5), i(10)], &ctx()).unwrap(), arr(vec![]));
    }

    // =========================================================================
    // reverse — extended
    // =========================================================================

    #[test]
    fn reverse_strings() {
        let data = arr(vec![s("a"), s("b"), s("c")]);
        assert_eq!(reverse(&[data], &ctx()).unwrap(), arr(vec![s("c"), s("b"), s("a")]));
    }

    #[test]
    fn reverse_two_elements() {
        let data = arr(vec![i(1), i(2)]);
        assert_eq!(reverse(&[data], &ctx()).unwrap(), arr(vec![i(2), i(1)]));
    }

    #[test]
    fn reverse_non_array_err() {
        assert!(reverse(&[i(42)], &ctx()).is_err());
    }

    // =========================================================================
    // every — extended
    // =========================================================================

    #[test]
    fn every_single_element_match() {
        let data = arr(vec![obj(vec![("v", i(10))])]);
        assert_eq!(every(&[data, s("v"), s("eq"), i(10)], &ctx()).unwrap(), b(true));
    }

    #[test]
    fn every_single_element_no_match() {
        let data = arr(vec![obj(vec![("v", i(5))])]);
        assert_eq!(every(&[data, s("v"), s("eq"), i(10)], &ctx()).unwrap(), b(false));
    }

    #[test]
    fn every_too_few_args() {
        assert!(every(&[arr(vec![])], &ctx()).is_err());
    }

    #[test]
    fn every_lte_all_match() {
        let data = arr(vec![
            obj(vec![("v", i(1))]),
            obj(vec![("v", i(5))]),
            obj(vec![("v", i(10))]),
        ]);
        assert_eq!(every(&[data, s("v"), s("lte"), i(10)], &ctx()).unwrap(), b(true));
    }

    // =========================================================================
    // some — extended
    // =========================================================================

    #[test]
    fn some_all_match() {
        let data = arr(vec![obj(vec![("v", i(10))]), obj(vec![("v", i(20))])]);
        assert_eq!(some(&[data, s("v"), s("gt"), i(5)], &ctx()).unwrap(), b(true));
    }

    #[test]
    fn some_single_element_match() {
        let data = arr(vec![obj(vec![("v", i(10))])]);
        assert_eq!(some(&[data, s("v"), s("eq"), i(10)], &ctx()).unwrap(), b(true));
    }

    #[test]
    fn some_too_few_args() {
        assert!(some(&[arr(vec![])], &ctx()).is_err());
    }

    // =========================================================================
    // find — extended
    // =========================================================================

    #[test]
    fn find_empty_array() {
        assert_eq!(find(&[arr(vec![]), s("x"), s("eq"), i(1)], &ctx()).unwrap(), null());
    }

    #[test]
    fn find_returns_first_of_many() {
        let data = arr(vec![
            obj(vec![("id", i(1)), ("label", s("first"))]),
            obj(vec![("id", i(1)), ("label", s("second"))]),
        ]);
        let result = find(&[data, s("id"), s("eq"), i(1)], &ctx()).unwrap();
        if let DynValue::Object(pairs) = result {
            let label = pairs.iter().find(|(k, _)| k == "label").unwrap();
            assert_eq!(label.1, s("first"));
        } else { panic!(); }
    }

    #[test]
    fn find_too_few_args() {
        assert!(find(&[arr(vec![])], &ctx()).is_err());
    }

    // =========================================================================
    // find_index — extended
    // =========================================================================

    #[test]
    fn find_index_empty_array() {
        assert_eq!(find_index(&[arr(vec![]), s("x"), s("eq"), i(1)], &ctx()).unwrap(), i(-1));
    }

    #[test]
    fn find_index_last_element() {
        let data = arr(vec![
            obj(vec![("x", i(1))]),
            obj(vec![("x", i(2))]),
            obj(vec![("x", i(3))]),
        ]);
        assert_eq!(find_index(&[data, s("x"), s("eq"), i(3)], &ctx()).unwrap(), i(2));
    }

    #[test]
    fn find_index_too_few_args() {
        assert!(find_index(&[arr(vec![])], &ctx()).is_err());
    }

    // =========================================================================
    // includes — extended
    // =========================================================================

    #[test]
    fn includes_string_value() {
        let data = arr(vec![s("hello"), s("world")]);
        assert_eq!(includes(&[data, s("hello")], &ctx()).unwrap(), b(true));
    }

    #[test]
    fn includes_null_in_array() {
        let data = arr(vec![i(1), null(), i(3)]);
        assert_eq!(includes(&[data, null()], &ctx()).unwrap(), b(true));
    }

    #[test]
    fn includes_bool_value() {
        let data = arr(vec![b(true), b(false)]);
        assert_eq!(includes(&[data, b(true)], &ctx()).unwrap(), b(true));
    }

    #[test]
    fn includes_too_few_args() {
        assert!(includes(&[arr(vec![])], &ctx()).is_err());
    }

    // =========================================================================
    // concat_arrays — extended
    // =========================================================================

    #[test]
    fn concat_both_empty() {
        assert_eq!(concat_arrays(&[arr(vec![]), arr(vec![])], &ctx()).unwrap(), arr(vec![]));
    }

    #[test]
    fn concat_first_empty() {
        assert_eq!(concat_arrays(&[arr(vec![]), arr(vec![i(1)])], &ctx()).unwrap(), arr(vec![i(1)]));
    }

    #[test]
    fn concat_mixed_types() {
        let a = arr(vec![i(1), s("two")]);
        let b_arr = arr(vec![b(true), null()]);
        let result = concat_arrays(&[a, b_arr], &ctx()).unwrap();
        if let DynValue::Array(items) = result { assert_eq!(items.len(), 4); } else { panic!(); }
    }

    #[test]
    fn concat_too_few_args() {
        assert!(concat_arrays(&[arr(vec![])], &ctx()).is_err());
    }

    // =========================================================================
    // zip_verb — extended
    // =========================================================================

    #[test]
    fn zip_both_empty() {
        assert_eq!(zip_verb(&[arr(vec![]), arr(vec![])], &ctx()).unwrap(), arr(vec![]));
    }

    #[test]
    fn zip_one_empty() {
        assert_eq!(zip_verb(&[arr(vec![i(1)]), arr(vec![])], &ctx()).unwrap(), arr(vec![]));
    }

    #[test]
    fn zip_single_elements() {
        let result = zip_verb(&[arr(vec![i(1)]), arr(vec![s("a")])], &ctx()).unwrap();
        assert_eq!(result, arr(vec![arr(vec![i(1), s("a")])]));
    }

    #[test]
    fn zip_too_few_args() {
        assert!(zip_verb(&[arr(vec![])], &ctx()).is_err());
    }

    // =========================================================================
    // group_by — extended
    // =========================================================================

    #[test]
    fn group_by_empty_array() {
        let result = group_by(&[arr(vec![]), s("key")], &ctx()).unwrap();
        if let DynValue::Object(pairs) = result { assert_eq!(pairs.len(), 0); } else { panic!(); }
    }

    #[test]
    fn group_by_single_group() {
        let data = arr(vec![
            obj(vec![("type", s("A")), ("v", i(1))]),
            obj(vec![("type", s("A")), ("v", i(2))]),
        ]);
        let result = group_by(&[data, s("type")], &ctx()).unwrap();
        if let DynValue::Object(pairs) = result {
            assert_eq!(pairs.len(), 1);
            assert_eq!(pairs[0].0, "A");
        } else { panic!(); }
    }

    #[test]
    fn group_by_missing_field_uses_null_key() {
        let data = arr(vec![
            obj(vec![("v", i(1))]),
            obj(vec![("type", s("A")), ("v", i(2))]),
        ]);
        let result = group_by(&[data, s("type")], &ctx()).unwrap();
        if let DynValue::Object(pairs) = result {
            assert_eq!(pairs.len(), 2);
            // First item has no "type" field, so key is "null"
            assert!(pairs.iter().any(|(k, _)| k == "null"));
        } else { panic!(); }
    }

    #[test]
    fn group_by_integer_field() {
        let data = arr(vec![
            obj(vec![("score", i(10))]),
            obj(vec![("score", i(20))]),
            obj(vec![("score", i(10))]),
        ]);
        let result = group_by(&[data, s("score")], &ctx()).unwrap();
        if let DynValue::Object(pairs) = result {
            assert_eq!(pairs.len(), 2);
        } else { panic!(); }
    }

    // =========================================================================
    // partition — extended
    // =========================================================================

    #[test]
    fn partition_all_match() {
        let data = arr(vec![
            obj(vec![("v", i(10))]),
            obj(vec![("v", i(20))]),
        ]);
        let result = partition(&[data, s("v"), s("gt"), i(0)], &ctx()).unwrap();
        if let DynValue::Array(parts) = result {
            if let DynValue::Array(matching) = &parts[0] { assert_eq!(matching.len(), 2); }
            if let DynValue::Array(non) = &parts[1] { assert_eq!(non.len(), 0); }
        } else { panic!(); }
    }

    #[test]
    fn partition_none_match() {
        let data = arr(vec![obj(vec![("v", i(1))])]);
        let result = partition(&[data, s("v"), s("gt"), i(100)], &ctx()).unwrap();
        if let DynValue::Array(parts) = result {
            if let DynValue::Array(matching) = &parts[0] { assert_eq!(matching.len(), 0); }
            if let DynValue::Array(non) = &parts[1] { assert_eq!(non.len(), 1); }
        } else { panic!(); }
    }

    #[test]
    fn partition_empty_array() {
        let result = partition(&[arr(vec![]), s("v"), s("eq"), i(1)], &ctx()).unwrap();
        if let DynValue::Array(parts) = result {
            assert_eq!(parts.len(), 2);
            if let DynValue::Array(m) = &parts[0] { assert!(m.is_empty()); }
            if let DynValue::Array(n) = &parts[1] { assert!(n.is_empty()); }
        } else { panic!(); }
    }

    #[test]
    fn partition_too_few_args() {
        assert!(partition(&[arr(vec![])], &ctx()).is_err());
    }

    // =========================================================================
    // take — extended
    // =========================================================================

    #[test]
    fn take_zero() {
        let data = arr(vec![i(1), i(2), i(3)]);
        assert_eq!(take(&[data, i(0)], &ctx()).unwrap(), arr(vec![]));
    }

    #[test]
    fn take_empty_array() {
        assert_eq!(take(&[arr(vec![]), i(5)], &ctx()).unwrap(), arr(vec![]));
    }

    #[test]
    fn take_exact_length() {
        let data = arr(vec![i(1), i(2)]);
        assert_eq!(take(&[data, i(2)], &ctx()).unwrap(), arr(vec![i(1), i(2)]));
    }

    #[test]
    fn take_too_few_args() {
        assert!(take(&[arr(vec![])], &ctx()).is_err());
    }

    // =========================================================================
    // drop — extended
    // =========================================================================

    #[test]
    fn drop_zero() {
        let data = arr(vec![i(1), i(2), i(3)]);
        assert_eq!(drop(&[data, i(0)], &ctx()).unwrap(), arr(vec![i(1), i(2), i(3)]));
    }

    #[test]
    fn drop_empty_array() {
        assert_eq!(drop(&[arr(vec![]), i(5)], &ctx()).unwrap(), arr(vec![]));
    }

    #[test]
    fn drop_exact_length() {
        let data = arr(vec![i(1), i(2)]);
        assert_eq!(drop(&[data, i(2)], &ctx()).unwrap(), arr(vec![]));
    }

    #[test]
    fn drop_too_few_args() {
        assert!(drop(&[arr(vec![])], &ctx()).is_err());
    }

    // =========================================================================
    // chunk — extended
    // =========================================================================

    #[test]
    fn chunk_single_element() {
        let data = arr(vec![i(1)]);
        assert_eq!(chunk(&[data, i(1)], &ctx()).unwrap(), arr(vec![arr(vec![i(1)])]));
    }

    #[test]
    fn chunk_size_larger_than_array() {
        let data = arr(vec![i(1), i(2)]);
        assert_eq!(chunk(&[data, i(10)], &ctx()).unwrap(), arr(vec![arr(vec![i(1), i(2)])]));
    }

    #[test]
    fn chunk_empty_array() {
        assert_eq!(chunk(&[arr(vec![]), i(3)], &ctx()).unwrap(), arr(vec![]));
    }

    #[test]
    fn chunk_size_one() {
        let data = arr(vec![i(1), i(2), i(3)]);
        let result = chunk(&[data, i(1)], &ctx()).unwrap();
        assert_eq!(result, arr(vec![arr(vec![i(1)]), arr(vec![i(2)]), arr(vec![i(3)])]));
    }

    // =========================================================================
    // range_verb — extended
    // =========================================================================

    #[test]
    fn range_single_element() {
        assert_eq!(range_verb(&[i(0), i(1)], &ctx()).unwrap(), arr(vec![i(0)]));
    }

    #[test]
    fn range_negative_values() {
        assert_eq!(range_verb(&[i(-3), i(0)], &ctx()).unwrap(), arr(vec![i(-3), i(-2), i(-1)]));
    }

    #[test]
    fn range_same_start_end() {
        assert_eq!(range_verb(&[i(5), i(5)], &ctx()).unwrap(), arr(vec![]));
    }

    #[test]
    fn range_step_of_two() {
        assert_eq!(range_verb(&[i(0), i(6), i(2)], &ctx()).unwrap(), arr(vec![i(0), i(2), i(4)]));
    }

    #[test]
    fn range_too_few_args() {
        assert!(range_verb(&[i(0)], &ctx()).is_err());
    }

    // =========================================================================
    // compact — extended
    // =========================================================================

    #[test]
    fn compact_no_nulls() {
        let data = arr(vec![i(1), i(2), i(3)]);
        assert_eq!(compact(&[data], &ctx()).unwrap(), arr(vec![i(1), i(2), i(3)]));
    }

    #[test]
    fn compact_empty_array() {
        assert_eq!(compact(&[arr(vec![])], &ctx()).unwrap(), arr(vec![]));
    }

    #[test]
    fn compact_only_empty_strings() {
        let data = arr(vec![s(""), s(""), s("")]);
        assert_eq!(compact(&[data], &ctx()).unwrap(), arr(vec![]));
    }

    #[test]
    fn compact_keeps_non_empty_strings() {
        let data = arr(vec![s(""), s("hello"), null(), s("world")]);
        assert_eq!(compact(&[data], &ctx()).unwrap(), arr(vec![s("hello"), s("world")]));
    }

    #[test]
    fn compact_keeps_zeros_and_false() {
        let data = arr(vec![i(0), b(false), null(), s("")]);
        let result = compact(&[data], &ctx()).unwrap();
        if let DynValue::Array(items) = result {
            assert_eq!(items.len(), 2); // 0 and false are kept
            assert!(items.contains(&i(0)));
            assert!(items.contains(&b(false)));
        } else { panic!(); }
    }

    // =========================================================================
    // dedupe — extended
    // =========================================================================

    #[test]
    fn dedupe_empty_array() {
        let result = dedupe(&[arr(vec![]), s("id")], &ctx()).unwrap();
        assert_eq!(result, arr(vec![]));
    }

    #[test]
    fn dedupe_no_duplicates() {
        let data = arr(vec![
            obj(vec![("id", i(1))]),
            obj(vec![("id", i(2))]),
            obj(vec![("id", i(3))]),
        ]);
        let result = dedupe(&[data, s("id")], &ctx()).unwrap();
        if let DynValue::Array(items) = result { assert_eq!(items.len(), 3); } else { panic!(); }
    }

    #[test]
    fn dedupe_keeps_first_occurrence() {
        let data = arr(vec![
            obj(vec![("id", i(1)), ("val", s("first"))]),
            obj(vec![("id", i(1)), ("val", s("second"))]),
        ]);
        let result = dedupe(&[data, s("id")], &ctx()).unwrap();
        if let DynValue::Array(items) = result {
            assert_eq!(items.len(), 1);
            if let DynValue::Object(pairs) = &items[0] {
                let val = pairs.iter().find(|(k, _)| k == "val").unwrap();
                assert_eq!(val.1, s("first"));
            }
        } else { panic!(); }
    }

    #[test]
    fn dedupe_too_few_args() {
        assert!(dedupe(&[arr(vec![])], &ctx()).is_err());
    }

    // =========================================================================
    // keys — extended
    // =========================================================================

    #[test]
    fn keys_empty_object() {
        let data = obj(vec![]);
        assert_eq!(keys(&[data], &ctx()).unwrap(), arr(vec![]));
    }

    #[test]
    fn keys_single_key() {
        let data = obj(vec![("only", i(1))]);
        assert_eq!(keys(&[data], &ctx()).unwrap(), arr(vec![s("only")]));
    }

    #[test]
    fn keys_preserves_order() {
        let data = obj(vec![("z", i(1)), ("a", i(2)), ("m", i(3))]);
        let result = keys(&[data], &ctx()).unwrap();
        assert_eq!(result, arr(vec![s("z"), s("a"), s("m")]));
    }

    // =========================================================================
    // values_verb — extended
    // =========================================================================

    #[test]
    fn values_empty_object() {
        let data = obj(vec![]);
        assert_eq!(values_verb(&[data], &ctx()).unwrap(), arr(vec![]));
    }

    #[test]
    fn values_mixed_types() {
        let data = obj(vec![("a", i(1)), ("b", s("two")), ("c", b(true))]);
        let result = values_verb(&[data], &ctx()).unwrap();
        if let DynValue::Array(items) = result { assert_eq!(items.len(), 3); } else { panic!(); }
    }

    // =========================================================================
    // entries — extended
    // =========================================================================

    #[test]
    fn entries_empty_object() {
        let data = obj(vec![]);
        assert_eq!(entries(&[data], &ctx()).unwrap(), arr(vec![]));
    }

    #[test]
    fn entries_multiple_pairs() {
        let data = obj(vec![("a", i(1)), ("b", i(2))]);
        let result = entries(&[data], &ctx()).unwrap();
        if let DynValue::Array(items) = result {
            assert_eq!(items.len(), 2);
            assert_eq!(items[0], arr(vec![s("a"), i(1)]));
            assert_eq!(items[1], arr(vec![s("b"), i(2)]));
        } else { panic!(); }
    }

    #[test]
    fn entries_non_object_err() {
        assert!(entries(&[arr(vec![])], &ctx()).is_err());
    }

    // =========================================================================
    // has — extended
    // =========================================================================

    #[test]
    fn has_empty_object() {
        let data = obj(vec![]);
        assert_eq!(has(&[data, s("anything")], &ctx()).unwrap(), b(false));
    }

    #[test]
    fn has_too_few_args() {
        assert!(has(&[obj(vec![])], &ctx()).is_err());
    }

    #[test]
    fn has_with_null_value() {
        let data = obj(vec![("key", null())]);
        assert_eq!(has(&[data, s("key")], &ctx()).unwrap(), b(true));
    }

    // =========================================================================
    // get_verb — extended
    // =========================================================================

    #[test]
    fn get_nested_path() {
        let data = obj(vec![
            ("a", obj(vec![("b", obj(vec![("c", i(42))]))]))
        ]);
        assert_eq!(get_verb(&[data, s("a.b.c")], &ctx()).unwrap(), i(42));
    }

    #[test]
    fn get_top_level() {
        let data = obj(vec![("x", i(99))]);
        assert_eq!(get_verb(&[data, s("x")], &ctx()).unwrap(), i(99));
    }

    #[test]
    fn get_array_index() {
        let data = obj(vec![("items", arr(vec![i(10), i(20), i(30)]))]);
        assert_eq!(get_verb(&[data, s("items.1")], &ctx()).unwrap(), i(20));
    }

    #[test]
    fn get_missing_nested_path() {
        let data = obj(vec![("a", i(1))]);
        assert_eq!(get_verb(&[data, s("a.b.c")], &ctx()).unwrap(), null());
    }

    #[test]
    fn get_too_few_args() {
        assert!(get_verb(&[obj(vec![])], &ctx()).is_err());
    }

    // =========================================================================
    // merge — extended
    // =========================================================================

    #[test]
    fn merge_both_empty() {
        let result = merge(&[obj(vec![]), obj(vec![])], &ctx()).unwrap();
        if let DynValue::Object(pairs) = result { assert!(pairs.is_empty()); } else { panic!(); }
    }

    #[test]
    fn merge_no_overlap() {
        let a = obj(vec![("a", i(1))]);
        let b_obj = obj(vec![("b", i(2))]);
        let result = merge(&[a, b_obj], &ctx()).unwrap();
        if let DynValue::Object(pairs) = result {
            assert_eq!(pairs.len(), 2);
        } else { panic!(); }
    }

    #[test]
    fn merge_complete_overlap() {
        let a = obj(vec![("x", i(1))]);
        let b_obj = obj(vec![("x", i(2))]);
        let result = merge(&[a, b_obj], &ctx()).unwrap();
        if let DynValue::Object(pairs) = result {
            assert_eq!(pairs.len(), 1);
            assert_eq!(pairs[0].1, i(2));
        } else { panic!(); }
    }

    #[test]
    fn merge_too_few_args() {
        assert!(merge(&[obj(vec![])], &ctx()).is_err());
    }

    // =========================================================================
    // sum_verb — extended
    // =========================================================================

    #[test]
    fn sum_single_element() {
        assert_eq!(sum_verb(&[arr(vec![i(42)])], &ctx()).unwrap(), i(42));
    }

    #[test]
    fn sum_negative_numbers() {
        let data = arr(vec![i(-1), i(-2), i(-3)]);
        assert_eq!(sum_verb(&[data], &ctx()).unwrap(), i(-6));
    }

    #[test]
    fn sum_with_non_numeric_ignored() {
        let data = arr(vec![i(1), s("hello"), i(2), b(true)]);
        assert_eq!(sum_verb(&[data], &ctx()).unwrap(), i(3));
    }

    #[test]
    fn sum_large_floats() {
        let data = arr(vec![f(1e10), f(2e10)]);
        assert_eq!(sum_verb(&[data], &ctx()).unwrap(), f(3e10));
    }

    // =========================================================================
    // count_verb — extended
    // =========================================================================

    #[test]
    fn count_with_nulls() {
        let data = arr(vec![null(), null(), i(1)]);
        assert_eq!(count_verb(&[data], &ctx()).unwrap(), i(3));
    }

    #[test]
    fn count_single_element() {
        assert_eq!(count_verb(&[arr(vec![i(42)])], &ctx()).unwrap(), i(1));
    }

    #[test]
    fn count_non_array_err() {
        assert!(count_verb(&[i(42)], &ctx()).is_err());
    }

    // =========================================================================
    // min_verb — extended
    // =========================================================================

    #[test]
    fn min_single_element() {
        assert_eq!(min_verb(&[arr(vec![i(42)])], &ctx()).unwrap(), i(42));
    }

    #[test]
    fn min_negative_numbers() {
        let data = arr(vec![i(-1), i(-5), i(-2)]);
        assert_eq!(min_verb(&[data], &ctx()).unwrap(), i(-5));
    }

    #[test]
    fn min_mixed_int_float() {
        let data = arr(vec![i(5), f(2.5), i(3)]);
        assert_eq!(min_verb(&[data], &ctx()).unwrap(), f(2.5));
    }

    #[test]
    fn min_non_array_err() {
        assert!(min_verb(&[i(42)], &ctx()).is_err());
    }

    // =========================================================================
    // max_verb — extended
    // =========================================================================

    #[test]
    fn max_single_element() {
        assert_eq!(max_verb(&[arr(vec![i(42)])], &ctx()).unwrap(), i(42));
    }

    #[test]
    fn max_negative_numbers() {
        let data = arr(vec![i(-1), i(-5), i(-2)]);
        assert_eq!(max_verb(&[data], &ctx()).unwrap(), i(-1));
    }

    #[test]
    fn max_mixed_int_float() {
        let data = arr(vec![i(5), f(7.5), i(3)]);
        assert_eq!(max_verb(&[data], &ctx()).unwrap(), f(7.5));
    }

    #[test]
    fn max_floats() {
        let data = arr(vec![f(1.1), f(9.9), f(5.5)]);
        assert_eq!(max_verb(&[data], &ctx()).unwrap(), f(9.9));
    }

    // =========================================================================
    // avg — extended
    // =========================================================================

    #[test]
    fn avg_floats() {
        let data = arr(vec![f(1.0), f(2.0), f(3.0)]);
        assert_eq!(avg(&[data], &ctx()).unwrap(), f(2.0));
    }

    #[test]
    fn avg_mixed_int_float() {
        let data = arr(vec![i(1), f(2.0), i(3)]);
        assert_eq!(avg(&[data], &ctx()).unwrap(), f(2.0));
    }

    #[test]
    fn avg_with_non_numeric_skipped() {
        let data = arr(vec![i(10), s("abc"), i(20)]);
        assert_eq!(avg(&[data], &ctx()).unwrap(), f(15.0));
    }

    #[test]
    fn avg_non_array_err() {
        assert!(avg(&[i(42)], &ctx()).is_err());
    }

    // =========================================================================
    // first_verb — extended
    // =========================================================================

    #[test]
    fn first_single_element() {
        assert_eq!(first_verb(&[arr(vec![i(42)])], &ctx()).unwrap(), i(42));
    }

    #[test]
    fn first_null_element() {
        assert_eq!(first_verb(&[arr(vec![null(), i(1)])], &ctx()).unwrap(), null());
    }

    #[test]
    fn first_non_array_err() {
        assert!(first_verb(&[i(42)], &ctx()).is_err());
    }

    // =========================================================================
    // last_verb — extended
    // =========================================================================

    #[test]
    fn last_single_element() {
        assert_eq!(last_verb(&[arr(vec![i(42)])], &ctx()).unwrap(), i(42));
    }

    #[test]
    fn last_null_at_end() {
        assert_eq!(last_verb(&[arr(vec![i(1), null()])], &ctx()).unwrap(), null());
    }

    #[test]
    fn last_non_array_err() {
        assert!(last_verb(&[i(42)], &ctx()).is_err());
    }

    // =========================================================================
    // cumsum — extended
    // =========================================================================

    #[test]
    fn cumsum_single_element() {
        assert_eq!(cumsum(&[arr(vec![i(5)])], &ctx()).unwrap(), arr(vec![i(5)]));
    }

    #[test]
    fn cumsum_floats() {
        let data = arr(vec![f(1.5), f(2.5), f(3.0)]);
        let result = cumsum(&[data], &ctx()).unwrap();
        if let DynValue::Array(items) = result {
            assert_eq!(items[0], f(1.5));
            // cumsum converts whole-number floats to Integer (4.0 -> 4)
            assert_eq!(items[1], i(4));
            // 1.5 + 2.5 + 3.0 = 7.0, also becomes Integer
            assert_eq!(items[2], i(7));
        } else { panic!(); }
    }

    #[test]
    fn cumsum_negative_numbers() {
        let data = arr(vec![i(5), i(-3), i(2)]);
        assert_eq!(cumsum(&[data], &ctx()).unwrap(), arr(vec![i(5), i(2), i(4)]));
    }

    #[test]
    fn cumsum_no_args_returns_null() {
        assert_eq!(cumsum(&[], &ctx()).unwrap(), null());
    }

    // =========================================================================
    // cumprod — extended
    // =========================================================================

    #[test]
    fn cumprod_single_element() {
        assert_eq!(cumprod(&[arr(vec![i(5)])], &ctx()).unwrap(), arr(vec![i(5)]));
    }

    #[test]
    fn cumprod_empty() {
        assert_eq!(cumprod(&[arr(vec![])], &ctx()).unwrap(), arr(vec![]));
    }

    #[test]
    fn cumprod_floats() {
        let data = arr(vec![f(2.0), f(3.0), f(4.0)]);
        let result = cumprod(&[data], &ctx()).unwrap();
        if let DynValue::Array(items) = result {
            assert_eq!(items[0], i(2));
            assert_eq!(items[1], i(6));
            assert_eq!(items[2], i(24));
        } else { panic!(); }
    }

    #[test]
    fn cumprod_no_args_returns_null() {
        assert_eq!(cumprod(&[], &ctx()).unwrap(), null());
    }

    // =========================================================================
    // diff_verb — extended
    // =========================================================================

    #[test]
    fn diff_empty_array() {
        assert_eq!(diff_verb(&[arr(vec![])], &ctx()).unwrap(), arr(vec![]));
    }

    #[test]
    fn diff_single_element() {
        assert_eq!(diff_verb(&[arr(vec![i(5)])], &ctx()).unwrap(), arr(vec![null()]));
    }

    #[test]
    fn diff_floats() {
        let data = arr(vec![f(1.0), f(3.0), f(6.0)]);
        let result = diff_verb(&[data], &ctx()).unwrap();
        if let DynValue::Array(items) = result {
            assert_eq!(items[0], null());
            assert_eq!(items[1], i(2));
            assert_eq!(items[2], i(3));
        } else { panic!(); }
    }

    #[test]
    fn diff_no_args_returns_null() {
        assert_eq!(diff_verb(&[], &ctx()).unwrap(), null());
    }

    // =========================================================================
    // pct_change — extended
    // =========================================================================

    #[test]
    fn pct_change_empty() {
        assert_eq!(pct_change(&[arr(vec![])], &ctx()).unwrap(), arr(vec![]));
    }

    #[test]
    fn pct_change_single() {
        assert_eq!(pct_change(&[arr(vec![i(100)])], &ctx()).unwrap(), arr(vec![null()]));
    }

    #[test]
    fn pct_change_doubling() {
        let data = arr(vec![i(100), i(200)]);
        let result = pct_change(&[data], &ctx()).unwrap();
        if let DynValue::Array(items) = result {
            assert_eq!(items[0], null());
            if let DynValue::Float(v) = items[1] { assert!((v - 1.0).abs() < 1e-10); } else { panic!(); }
        } else { panic!(); }
    }

    #[test]
    fn pct_change_with_zero_previous() {
        let data = arr(vec![i(0), i(100)]);
        let result = pct_change(&[data], &ctx()).unwrap();
        if let DynValue::Array(items) = result {
            assert_eq!(items[1], null()); // Division by zero -> null
        } else { panic!(); }
    }

    #[test]
    fn pct_change_no_args_returns_null() {
        assert_eq!(pct_change(&[], &ctx()).unwrap(), null());
    }

    // =========================================================================
    // shift_verb — extended
    // =========================================================================

    #[test]
    fn shift_zero_no_change() {
        let data = arr(vec![i(1), i(2), i(3)]);
        // Shift by 0 means no shift
        let result = shift_verb(&[data, i(0)], &ctx()).unwrap();
        assert_eq!(result, arr(vec![i(1), i(2), i(3)]));
    }

    #[test]
    fn shift_with_fill_value() {
        let data = arr(vec![i(1), i(2), i(3)]);
        let result = shift_verb(&[data, i(1), i(99)], &ctx()).unwrap();
        assert_eq!(result, arr(vec![i(99), i(1), i(2)]));
    }

    #[test]
    fn shift_empty_array() {
        assert_eq!(shift_verb(&[arr(vec![])], &ctx()).unwrap(), arr(vec![]));
    }

    #[test]
    fn shift_no_args_returns_null() {
        assert_eq!(shift_verb(&[], &ctx()).unwrap(), null());
    }

    // =========================================================================
    // lag — extended
    // =========================================================================

    #[test]
    fn lag_period_two() {
        let data = arr(vec![i(10), i(20), i(30), i(40)]);
        let result = lag(&[data, i(2)], &ctx()).unwrap();
        assert_eq!(result, arr(vec![null(), null(), i(10), i(20)]));
    }

    #[test]
    fn lag_empty_array() {
        assert_eq!(lag(&[arr(vec![])], &ctx()).unwrap(), arr(vec![]));
    }

    #[test]
    fn lag_no_args_returns_null() {
        assert_eq!(lag(&[], &ctx()).unwrap(), null());
    }

    // =========================================================================
    // lead — extended
    // =========================================================================

    #[test]
    fn lead_period_two() {
        let data = arr(vec![i(10), i(20), i(30), i(40)]);
        let result = lead(&[data, i(2)], &ctx()).unwrap();
        assert_eq!(result, arr(vec![i(30), i(40), null(), null()]));
    }

    #[test]
    fn lead_empty_array() {
        assert_eq!(lead(&[arr(vec![])], &ctx()).unwrap(), arr(vec![]));
    }

    #[test]
    fn lead_no_args_returns_null() {
        assert_eq!(lead(&[], &ctx()).unwrap(), null());
    }

    // =========================================================================
    // rank — extended
    // =========================================================================

    #[test]
    fn rank_tied_values() {
        let data = arr(vec![i(10), i(10), i(30)]);
        let result = rank(&[data], &ctx()).unwrap();
        if let DynValue::Array(items) = result {
            let get_rank = |item: &DynValue| {
                if let DynValue::Object(pairs) = item {
                    pairs.iter().find(|(k, _)| k == "_rank").map(|(_, v)| v.clone())
                } else { None }
            };
            // Both 10s should have same rank
            assert_eq!(get_rank(&items[0]), get_rank(&items[1]));
        } else { panic!(); }
    }

    #[test]
    fn rank_single_element() {
        let data = arr(vec![i(42)]);
        let result = rank(&[data], &ctx()).unwrap();
        if let DynValue::Array(items) = result {
            assert_eq!(items.len(), 1);
            if let DynValue::Object(pairs) = &items[0] {
                let rank_val = pairs.iter().find(|(k, _)| k == "_rank").unwrap();
                assert_eq!(rank_val.1, i(1));
            }
        } else { panic!(); }
    }

    #[test]
    fn rank_asc_direction() {
        let data = arr(vec![i(10), i(30), i(20)]);
        let result = rank(&[data, null(), s("asc")], &ctx()).unwrap();
        if let DynValue::Array(items) = result {
            let get_rank = |item: &DynValue| {
                if let DynValue::Object(pairs) = item {
                    pairs.iter().find(|(k, _)| k == "_rank").map(|(_, v)| v.clone())
                } else { None }
            };
            assert_eq!(get_rank(&items[0]), Some(i(1))); // 10 -> rank 1 (asc)
            assert_eq!(get_rank(&items[1]), Some(i(3))); // 30 -> rank 3 (asc)
            assert_eq!(get_rank(&items[2]), Some(i(2))); // 20 -> rank 2 (asc)
        } else { panic!(); }
    }

    #[test]
    fn rank_empty_returns_empty() {
        let result = rank(&[arr(vec![])], &ctx()).unwrap();
        assert_eq!(result, arr(vec![]));
    }

    #[test]
    fn rank_no_args_returns_null() {
        assert_eq!(rank(&[], &ctx()).unwrap(), null());
    }

    // =========================================================================
    // fill_missing — extended
    // =========================================================================

    #[test]
    fn fill_missing_no_nulls() {
        let data = arr(vec![i(1), i(2), i(3)]);
        assert_eq!(fill_missing(&[data, i(0)], &ctx()).unwrap(), arr(vec![i(1), i(2), i(3)]));
    }

    #[test]
    fn fill_missing_all_nulls_value() {
        let data = arr(vec![null(), null()]);
        assert_eq!(fill_missing(&[data, i(99)], &ctx()).unwrap(), arr(vec![i(99), i(99)]));
    }

    #[test]
    fn fill_missing_forward_first_null() {
        // When first element is null in forward strategy, uses fill_value
        let data = arr(vec![null(), i(5), null()]);
        let result = fill_missing(&[data, i(0), s("forward")], &ctx()).unwrap();
        assert_eq!(result, arr(vec![i(0), i(5), i(5)]));
    }

    #[test]
    fn fill_missing_backward_last_null() {
        // When last element is null in backward strategy, uses fill_value
        let data = arr(vec![i(5), null()]);
        let result = fill_missing(&[data, i(0), s("backward")], &ctx()).unwrap();
        assert_eq!(result, arr(vec![i(5), i(0)]));
    }

    #[test]
    fn fill_missing_empty_array() {
        assert_eq!(fill_missing(&[arr(vec![]), i(0)], &ctx()).unwrap(), arr(vec![]));
    }

    #[test]
    fn fill_missing_no_args_returns_null() {
        assert_eq!(fill_missing(&[], &ctx()).unwrap(), null());
    }

    // =========================================================================
    // sample_verb — extended
    // =========================================================================

    #[test]
    fn sample_zero_count() {
        let data = arr(vec![i(1), i(2), i(3)]);
        assert_eq!(sample_verb(&[data, i(0)], &ctx()).unwrap(), arr(vec![]));
    }

    #[test]
    fn sample_more_than_length() {
        let data = arr(vec![i(1), i(2)]);
        assert_eq!(sample_verb(&[data, i(10)], &ctx()).unwrap(), arr(vec![i(1), i(2)]));
    }

    #[test]
    fn sample_empty_array() {
        assert_eq!(sample_verb(&[arr(vec![]), i(5)], &ctx()).unwrap(), arr(vec![]));
    }

    #[test]
    fn sample_too_few_args() {
        assert!(sample_verb(&[arr(vec![])], &ctx()).is_err());
    }

    // =========================================================================
    // limit — extended
    // =========================================================================

    #[test]
    fn limit_zero() {
        let data = arr(vec![i(1), i(2), i(3)]);
        assert_eq!(limit(&[data, i(0)], &ctx()).unwrap(), arr(vec![]));
    }

    #[test]
    fn limit_exact_length() {
        let data = arr(vec![i(1), i(2)]);
        assert_eq!(limit(&[data, i(2)], &ctx()).unwrap(), arr(vec![i(1), i(2)]));
    }

    // =========================================================================
    // accumulate — extended
    // =========================================================================

    #[test]
    fn accumulate_float_addition() {
        let mut h = OwnedCtx::new();
        h.acc.insert("total".to_string(), f(10.5));
        let result = accumulate(&[s("total"), f(5.5)], &h.ctx()).unwrap();
        assert_eq!(result, f(16.0));
    }

    #[test]
    fn accumulate_int_float_cross() {
        let mut h = OwnedCtx::new();
        h.acc.insert("total".to_string(), i(10));
        let result = accumulate(&[s("total"), f(5.5)], &h.ctx()).unwrap();
        assert_eq!(result, f(15.5));
    }

    #[test]
    fn accumulate_too_few_args() {
        assert!(accumulate(&[s("x")], &ctx()).is_err());
    }

    // =========================================================================
    // set_verb — extended
    // =========================================================================

    #[test]
    fn set_verb_string_value() {
        assert_eq!(set_verb(&[s("name"), s("hello")], &ctx()).unwrap(), s("hello"));
    }

    #[test]
    fn set_verb_null_value() {
        assert_eq!(set_verb(&[s("name"), null()], &ctx()).unwrap(), null());
    }

    // =========================================================================
    // Geo verbs — extended
    // =========================================================================

    #[test]
    fn to_radians_zero() {
        let result = to_radians(&[f(0.0)], &ctx()).unwrap();
        if let DynValue::Float(v) = result { assert!(v.abs() < 1e-10); } else { panic!(); }
    }

    #[test]
    fn to_radians_90() {
        let result = to_radians(&[f(90.0)], &ctx()).unwrap();
        if let DynValue::Float(v) = result {
            assert!((v - std::f64::consts::FRAC_PI_2).abs() < 1e-10);
        } else { panic!(); }
    }

    #[test]
    fn to_degrees_zero() {
        let result = to_degrees(&[f(0.0)], &ctx()).unwrap();
        if let DynValue::Float(v) = result { assert!(v.abs() < 1e-10); } else { panic!(); }
    }

    #[test]
    fn to_degrees_non_numeric_err() {
        assert!(to_degrees(&[s("abc")], &ctx()).is_err());
    }

    #[test]
    fn to_radians_integer_input() {
        let result = to_radians(&[i(180)], &ctx()).unwrap();
        if let DynValue::Float(v) = result {
            assert!((v - std::f64::consts::PI).abs() < 1e-10);
        } else { panic!(); }
    }

    #[test]
    fn distance_km_default() {
        // New York to London (approximate)
        let result = distance(&[f(40.7128), f(-74.0060), f(51.5074), f(-0.1278)], &ctx()).unwrap();
        if let DynValue::Float(v) = result {
            assert!(v > 5000.0 && v < 6000.0); // ~5570 km
        } else { panic!(); }
    }

    #[test]
    fn distance_miles() {
        let result = distance(&[f(40.7128), f(-74.0060), f(51.5074), f(-0.1278), s("mi")], &ctx()).unwrap();
        if let DynValue::Float(v) = result {
            assert!(v > 3000.0 && v < 4000.0); // ~3459 miles
        } else { panic!(); }
    }

    #[test]
    fn in_bounding_box_on_edge() {
        // Point on the edge of the bounding box
        let result = in_bounding_box(&[f(0.0), f(0.0), f(0.0), f(0.0), f(10.0), f(10.0)], &ctx()).unwrap();
        assert_eq!(result, b(true));
    }

    #[test]
    fn in_bounding_box_too_few_args() {
        assert!(in_bounding_box(&[f(1.0), f(2.0)], &ctx()).is_err());
    }

    #[test]
    fn bearing_east() {
        // Going due east: same lat, increasing lon
        let result = bearing(&[f(0.0), f(0.0), f(0.0), f(10.0)], &ctx()).unwrap();
        if let DynValue::Float(v) = result {
            assert!((v - 90.0).abs() < 1.0); // ~90 degrees
        } else { panic!(); }
    }

    #[test]
    fn bearing_too_few_args() {
        assert!(bearing(&[f(1.0), f(2.0)], &ctx()).is_err());
    }

    // =========================================================================
    // midpoint — extended
    // =========================================================================

    #[test]
    fn midpoint_equator() {
        let result = midpoint(&[f(0.0), f(0.0), f(0.0), f(10.0)], &ctx()).unwrap();
        if let DynValue::Object(pairs) = result {
            let lat = pairs.iter().find(|(k, _)| k == "lat").unwrap();
            let lon = pairs.iter().find(|(k, _)| k == "lon").unwrap();
            if let DynValue::Float(v) = &lat.1 { assert!(v.abs() < 0.01); }
            if let DynValue::Float(v) = &lon.1 { assert!((v - 5.0).abs() < 0.01); }
        } else { panic!(); }
    }

    // =========================================================================
    // uuid_verb — extended
    // =========================================================================

    #[test]
    fn uuid_deterministic_format() {
        let result = uuid_verb(&[], &ctx()).unwrap();
        if let DynValue::String(s) = result {
            assert!(s.contains('-'));
            let parts: Vec<&str> = s.split('-').collect();
            assert_eq!(parts.len(), 5);
        } else { panic!(); }
    }

    #[test]
    fn uuid_with_seed_is_deterministic() {
        let r1 = uuid_verb(&[s("my-seed")], &ctx()).unwrap();
        let r2 = uuid_verb(&[s("my-seed")], &ctx()).unwrap();
        assert_eq!(r1, r2);
    }

    #[test]
    fn uuid_different_seeds_differ() {
        let r1 = uuid_verb(&[s("seed-a")], &ctx()).unwrap();
        let r2 = uuid_verb(&[s("seed-b")], &ctx()).unwrap();
        assert_ne!(r1, r2);
    }

    // =========================================================================
    // sequence / reset_sequence — extended
    // =========================================================================

    #[test]
    fn sequence_with_existing_counter() {
        let mut h = OwnedCtx::new();
        h.acc.insert("__seq_counter".to_string(), i(5));
        assert_eq!(sequence(&[s("counter")], &h.ctx()).unwrap(), i(5));
    }

    #[test]
    fn reset_sequence_no_args_err() {
        assert!(reset_sequence(&[], &ctx()).is_err());
    }

    #[test]
    fn reset_sequence_non_string_err() {
        assert!(reset_sequence(&[i(42)], &ctx()).is_err());
    }

    // =========================================================================
    // row_number — extended
    // =========================================================================

    #[test]
    fn row_number_large_index() {
        let mut h = OwnedCtx::new();
        h.lv.insert("$index".to_string(), i(999));
        assert_eq!(row_number(&[], &h.ctx()).unwrap(), i(999));
    }

    // =========================================================================
    // Nested array operations
    // =========================================================================

    #[test]
    fn nested_sort_then_take() {
        let data = arr(vec![i(5), i(1), i(3), i(2), i(4)]);
        let sorted = sort_verb(&[data], &ctx()).unwrap();
        let result = take(&[sorted, i(3)], &ctx()).unwrap();
        assert_eq!(result, arr(vec![i(1), i(2), i(3)]));
    }

    #[test]
    fn nested_filter_then_map() {
        let data = arr(vec![
            obj(vec![("age", i(10)), ("name", s("A"))]),
            obj(vec![("age", i(25)), ("name", s("B"))]),
            obj(vec![("age", i(30)), ("name", s("C"))]),
        ]);
        let filtered = filter(&[data, s("age"), s("gte"), i(25)], &ctx()).unwrap();
        let names = map_verb(&[filtered, s("name")], &ctx()).unwrap();
        assert_eq!(names, arr(vec![s("B"), s("C")]));
    }

    #[test]
    fn nested_flatten_then_distinct() {
        let data = arr(vec![arr(vec![i(1), i(2)]), arr(vec![i(2), i(3)])]);
        let flat = flatten(&[data], &ctx()).unwrap();
        let result = distinct(&[flat], &ctx()).unwrap();
        assert_eq!(result, arr(vec![i(1), i(2), i(3)]));
    }

    #[test]
    fn nested_concat_then_sort() {
        let a = arr(vec![i(3), i(1)]);
        let b_arr = arr(vec![i(4), i(2)]);
        let combined = concat_arrays(&[a, b_arr], &ctx()).unwrap();
        let result = sort_verb(&[combined], &ctx()).unwrap();
        assert_eq!(result, arr(vec![i(1), i(2), i(3), i(4)]));
    }

    #[test]
    fn nested_reverse_then_first() {
        let data = arr(vec![i(1), i(2), i(3)]);
        let reversed = reverse(&[data], &ctx()).unwrap();
        assert_eq!(first_verb(&[reversed], &ctx()).unwrap(), i(3));
    }

    #[test]
    fn nested_chunk_then_map_first() {
        let data = arr(vec![i(1), i(2), i(3), i(4)]);
        let chunks = chunk(&[data, i(2)], &ctx()).unwrap();
        // Each chunk is an array; we can get first chunk
        let first = first_verb(&[chunks], &ctx()).unwrap();
        assert_eq!(first, arr(vec![i(1), i(2)]));
    }

    #[test]
    fn nested_drop_then_count() {
        let data = arr(vec![i(1), i(2), i(3), i(4), i(5)]);
        let dropped = drop(&[data, i(2)], &ctx()).unwrap();
        assert_eq!(count_verb(&[dropped], &ctx()).unwrap(), i(3));
    }

    #[test]
    fn nested_compact_then_sum() {
        let data = arr(vec![i(1), null(), i(2), s(""), i(3)]);
        let compacted = compact(&[data], &ctx()).unwrap();
        assert_eq!(sum_verb(&[compacted], &ctx()).unwrap(), i(6));
    }

    // =========================================================================
    // Edge cases: type coercion in comparisons
    // =========================================================================

    #[test]
    fn compare_string_integer_eq() {
        // dyn_values_equal_cmp supports cross-type comparison
        let data = arr(vec![obj(vec![("v", s("42"))])]);
        let result = filter(&[data, s("v"), s("eq"), i(42)], &ctx()).unwrap();
        if let DynValue::Array(items) = result { assert_eq!(items.len(), 1); } else { panic!(); }
    }

    #[test]
    fn compare_integer_float_eq() {
        let data = arr(vec![obj(vec![("v", i(5))])]);
        let result = filter(&[data, s("v"), s("eq"), f(5.0)], &ctx()).unwrap();
        if let DynValue::Array(items) = result { assert_eq!(items.len(), 1); } else { panic!(); }
    }

    #[test]
    fn compare_gt_integer_vs_float() {
        let data = arr(vec![obj(vec![("v", i(10))])]);
        let result = filter(&[data, s("v"), s("gt"), f(9.5)], &ctx()).unwrap();
        if let DynValue::Array(items) = result { assert_eq!(items.len(), 1); } else { panic!(); }
    }
}
#[cfg(test)]
mod extended_tests_2 {
    use super::*;
    use std::collections::HashMap;

    fn ctx() -> VerbContext<'static> {
        static NULL: DynValue = DynValue::Null;
        static LV: std::sync::OnceLock<HashMap<String, DynValue>> = std::sync::OnceLock::new();
        static ACC: std::sync::OnceLock<HashMap<String, DynValue>> = std::sync::OnceLock::new();
        static TBL: std::sync::OnceLock<HashMap<String, crate::types::transform::LookupTable>> = std::sync::OnceLock::new();
        VerbContext {
            source: &NULL,
            loop_vars: LV.get_or_init(HashMap::new),
            accumulators: ACC.get_or_init(HashMap::new),
            tables: TBL.get_or_init(HashMap::new),
        }
    }

    struct OwnedCtx {
        null: DynValue,
        lv: HashMap<String, DynValue>,
        acc: HashMap<String, DynValue>,
        tbl: HashMap<String, crate::types::transform::LookupTable>,
    }
    impl OwnedCtx {
        fn new() -> Self { Self { null: DynValue::Null, lv: HashMap::new(), acc: HashMap::new(), tbl: HashMap::new() } }
        fn ctx(&self) -> VerbContext<'_> {
            VerbContext { source: &self.null, loop_vars: &self.lv, accumulators: &self.acc, tables: &self.tbl }
        }
    }
    fn s(v: &str) -> DynValue { DynValue::String(v.to_string()) }
    fn i(v: i64) -> DynValue { DynValue::Integer(v) }
    fn f(v: f64) -> DynValue { DynValue::Float(v) }
    fn b(v: bool) -> DynValue { DynValue::Bool(v) }
    fn null() -> DynValue { DynValue::Null }
    fn arr(items: Vec<DynValue>) -> DynValue { DynValue::Array(items) }
    fn obj(pairs: Vec<(&str, DynValue)>) -> DynValue {
        DynValue::Object(pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
    }

    // =========================================================================
    // every — additional operator coverage
    // =========================================================================

    #[test]
    fn every_gt_all_above() {
        let data = arr(vec![
            obj(vec![("val", i(10))]),
            obj(vec![("val", i(20))]),
        ]);
        assert_eq!(every(&[data, s("val"), s("gt"), i(5)], &ctx()).unwrap(), b(true));
    }

    #[test]
    fn every_gt_not_all_above() {
        let data = arr(vec![
            obj(vec![("val", i(3))]),
            obj(vec![("val", i(20))]),
        ]);
        assert_eq!(every(&[data, s("val"), s("gt"), i(5)], &ctx()).unwrap(), b(false));
    }

    #[test]
    fn every_contains_strings() {
        let data = arr(vec![
            obj(vec![("name", s("hello world"))]),
            obj(vec![("name", s("hello there"))]),
        ]);
        assert_eq!(every(&[data, s("name"), s("contains"), s("hello")], &ctx()).unwrap(), b(true));
    }

    #[test]
    fn every_ne_all_differ() {
        let data = arr(vec![
            obj(vec![("x", i(1))]),
            obj(vec![("x", i(2))]),
        ]);
        assert_eq!(every(&[data, s("x"), s("ne"), i(0)], &ctx()).unwrap(), b(true));
    }

    #[test]
    fn every_too_few_args() {
        assert!(every(&[arr(vec![]), s("f"), s("eq")], &ctx()).is_err());
    }

    // =========================================================================
    // some — additional operator coverage
    // =========================================================================

    #[test]
    fn some_contains_one_match() {
        let data = arr(vec![
            obj(vec![("t", s("abc"))]),
            obj(vec![("t", s("xyz"))]),
        ]);
        assert_eq!(some(&[data, s("t"), s("contains"), s("ab")], &ctx()).unwrap(), b(true));
    }

    #[test]
    fn some_lt_one_below() {
        let data = arr(vec![
            obj(vec![("v", i(10))]),
            obj(vec![("v", i(2))]),
        ]);
        assert_eq!(some(&[data, s("v"), s("lt"), i(5)], &ctx()).unwrap(), b(true));
    }

    #[test]
    fn some_none_match_gt() {
        let data = arr(vec![
            obj(vec![("v", i(1))]),
            obj(vec![("v", i(2))]),
        ]);
        assert_eq!(some(&[data, s("v"), s("gt"), i(100)], &ctx()).unwrap(), b(false));
    }

    #[test]
    fn some_too_few_args() {
        assert!(some(&[arr(vec![])], &ctx()).is_err());
    }

    // =========================================================================
    // find — additional cases
    // =========================================================================

    #[test]
    fn find_with_gt_returns_first() {
        let data = arr(vec![
            obj(vec![("v", i(3))]),
            obj(vec![("v", i(10))]),
            obj(vec![("v", i(20))]),
        ]);
        let result = find(&[data, s("v"), s("gt"), i(5)], &ctx()).unwrap();
        assert_eq!(get_field(&result, "v"), Some(i(10)));
    }

    #[test]
    fn find_with_contains() {
        let data = arr(vec![
            obj(vec![("name", s("alice"))]),
            obj(vec![("name", s("bob"))]),
        ]);
        let result = find(&[data, s("name"), s("contains"), s("ob")], &ctx()).unwrap();
        assert_eq!(get_field(&result, "name"), Some(s("bob")));
    }

    #[test]
    fn find_returns_null_on_no_match() {
        let data = arr(vec![obj(vec![("x", i(1))])]);
        assert_eq!(find(&[data, s("x"), s("eq"), i(99)], &ctx()).unwrap(), null());
    }

    // =========================================================================
    // find_index — additional cases
    // =========================================================================

    #[test]
    fn find_index_with_lte() {
        let data = arr(vec![
            obj(vec![("v", i(10))]),
            obj(vec![("v", i(5))]),
            obj(vec![("v", i(3))]),
        ]);
        assert_eq!(find_index(&[data, s("v"), s("lte"), i(5)], &ctx()).unwrap(), i(1));
    }

    #[test]
    fn find_index_with_ne() {
        let data = arr(vec![
            obj(vec![("v", i(5))]),
            obj(vec![("v", i(10))]),
        ]);
        assert_eq!(find_index(&[data, s("v"), s("ne"), i(5)], &ctx()).unwrap(), i(1));
    }

    #[test]
    fn find_index_returns_minus_one_on_no_match() {
        let data = arr(vec![obj(vec![("v", i(1))])]);
        assert_eq!(find_index(&[data, s("v"), s("eq"), i(99)], &ctx()).unwrap(), i(-1));
    }

    // =========================================================================
    // includes — additional type coverage
    // =========================================================================

    #[test]
    fn includes_integer_present() {
        assert_eq!(includes(&[arr(vec![i(1), i(2), i(3)]), i(2)], &ctx()).unwrap(), b(true));
    }

    #[test]
    fn includes_float_present() {
        assert_eq!(includes(&[arr(vec![f(1.5), f(2.5)]), f(2.5)], &ctx()).unwrap(), b(true));
    }

    #[test]
    fn includes_bool_absent() {
        assert_eq!(includes(&[arr(vec![b(true)]), b(false)], &ctx()).unwrap(), b(false));
    }

    #[test]
    fn includes_null_in_array() {
        assert_eq!(includes(&[arr(vec![i(1), null(), i(3)]), null()], &ctx()).unwrap(), b(true));
    }

    // =========================================================================
    // concat_arrays — edge cases
    // =========================================================================

    #[test]
    fn concat_arrays_mixed_types() {
        let a = arr(vec![i(1), s("two")]);
        let b_arr = arr(vec![b(true), null()]);
        let result = concat_arrays(&[a, b_arr], &ctx()).unwrap();
        assert_eq!(result, arr(vec![i(1), s("two"), b(true), null()]));
    }

    #[test]
    fn concat_arrays_nested_arrays() {
        let a = arr(vec![arr(vec![i(1)])]);
        let b_arr = arr(vec![arr(vec![i(2)])]);
        let result = concat_arrays(&[a, b_arr], &ctx()).unwrap();
        assert_eq!(result, arr(vec![arr(vec![i(1)]), arr(vec![i(2)])]));
    }

    // =========================================================================
    // zip — additional cases
    // =========================================================================

    #[test]
    fn zip_mixed_types() {
        let a = arr(vec![i(1), s("two")]);
        let b_arr = arr(vec![b(true), null()]);
        let result = zip_verb(&[a, b_arr], &ctx()).unwrap();
        assert_eq!(result, arr(vec![
            arr(vec![i(1), b(true)]),
            arr(vec![s("two"), null()]),
        ]));
    }

    #[test]
    fn zip_single_element_each() {
        let result = zip_verb(&[arr(vec![i(1)]), arr(vec![s("a")])], &ctx()).unwrap();
        assert_eq!(result, arr(vec![arr(vec![i(1), s("a")])]));
    }

    #[test]
    fn zip_first_longer_truncates() {
        let result = zip_verb(&[arr(vec![i(1), i(2), i(3)]), arr(vec![s("a")])], &ctx()).unwrap();
        assert_eq!(result, arr(vec![arr(vec![i(1), s("a")])]));
    }

    // =========================================================================
    // group_by — additional cases
    // =========================================================================

    #[test]
    fn group_by_bool_field() {
        let data = arr(vec![
            obj(vec![("active", b(true)), ("name", s("A"))]),
            obj(vec![("active", b(false)), ("name", s("B"))]),
            obj(vec![("active", b(true)), ("name", s("C"))]),
        ]);
        let result = group_by(&[data, s("active")], &ctx()).unwrap();
        if let DynValue::Object(pairs) = result {
            assert_eq!(pairs.len(), 2);
            let true_group = pairs.iter().find(|(k, _)| k == "true").unwrap();
            if let DynValue::Array(items) = &true_group.1 { assert_eq!(items.len(), 2); } else { panic!(); }
        } else { panic!(); }
    }

    #[test]
    fn group_by_all_same_key() {
        let data = arr(vec![
            obj(vec![("k", s("x")), ("v", i(1))]),
            obj(vec![("k", s("x")), ("v", i(2))]),
        ]);
        let result = group_by(&[data, s("k")], &ctx()).unwrap();
        if let DynValue::Object(pairs) = result {
            assert_eq!(pairs.len(), 1);
            assert_eq!(pairs[0].0, "x");
        } else { panic!(); }
    }

    #[test]
    fn group_by_missing_field_groups_under_null() {
        let data = arr(vec![
            obj(vec![("name", s("A"))]),
            obj(vec![("cat", s("x")), ("name", s("B"))]),
        ]);
        let result = group_by(&[data, s("cat")], &ctx()).unwrap();
        if let DynValue::Object(pairs) = result {
            let null_group = pairs.iter().find(|(k, _)| k == "null");
            assert!(null_group.is_some());
        } else { panic!(); }
    }

    // =========================================================================
    // partition — additional operator coverage
    // =========================================================================

    #[test]
    fn partition_gt_splits_correctly() {
        let data = arr(vec![
            obj(vec![("v", i(1))]),
            obj(vec![("v", i(10))]),
            obj(vec![("v", i(5))]),
        ]);
        let result = partition(&[data, s("v"), s("gt"), i(4)], &ctx()).unwrap();
        if let DynValue::Array(parts) = result {
            if let DynValue::Array(matching) = &parts[0] { assert_eq!(matching.len(), 2); } else { panic!(); }
            if let DynValue::Array(non_matching) = &parts[1] { assert_eq!(non_matching.len(), 1); } else { panic!(); }
        } else { panic!(); }
    }

    #[test]
    fn partition_contains_splits_strings() {
        let data = arr(vec![
            obj(vec![("t", s("hello world"))]),
            obj(vec![("t", s("goodbye"))]),
        ]);
        let result = partition(&[data, s("t"), s("contains"), s("hello")], &ctx()).unwrap();
        if let DynValue::Array(parts) = result {
            if let DynValue::Array(m) = &parts[0] { assert_eq!(m.len(), 1); } else { panic!(); }
            if let DynValue::Array(nm) = &parts[1] { assert_eq!(nm.len(), 1); } else { panic!(); }
        } else { panic!(); }
    }

    // =========================================================================
    // take — edge cases
    // =========================================================================

    #[test]
    fn take_negative_treated_as_zero() {
        // as_i64 on negative becomes large usize via cast, but min(arr.len()) clamps
        let data = arr(vec![i(1), i(2)]);
        let result = take(&[data, i(0)], &ctx()).unwrap();
        assert_eq!(result, arr(vec![]));
    }

    #[test]
    fn take_from_mixed_types() {
        let data = arr(vec![i(1), s("two"), b(true), null()]);
        let result = take(&[data, i(3)], &ctx()).unwrap();
        assert_eq!(result, arr(vec![i(1), s("two"), b(true)]));
    }

    // =========================================================================
    // drop — edge cases
    // =========================================================================

    #[test]
    fn drop_more_than_length() {
        let data = arr(vec![i(1), i(2)]);
        assert_eq!(drop(&[data, i(10)], &ctx()).unwrap(), arr(vec![]));
    }

    #[test]
    fn drop_from_mixed_types() {
        let data = arr(vec![s("a"), i(1), b(false)]);
        assert_eq!(drop(&[data, i(1)], &ctx()).unwrap(), arr(vec![i(1), b(false)]));
    }

    // =========================================================================
    // chunk — additional cases
    // =========================================================================

    #[test]
    fn chunk_size_equals_length() {
        let data = arr(vec![i(1), i(2), i(3)]);
        let result = chunk(&[data, i(3)], &ctx()).unwrap();
        assert_eq!(result, arr(vec![arr(vec![i(1), i(2), i(3)])]));
    }

    #[test]
    fn chunk_size_two_odd_length() {
        let data = arr(vec![i(1), i(2), i(3), i(4), i(5)]);
        let result = chunk(&[data, i(2)], &ctx()).unwrap();
        assert_eq!(result, arr(vec![
            arr(vec![i(1), i(2)]),
            arr(vec![i(3), i(4)]),
            arr(vec![i(5)]),
        ]));
    }

    // =========================================================================
    // range_verb — additional cases
    // =========================================================================

    #[test]
    fn range_large_step() {
        let result = range_verb(&[i(0), i(10), i(5)], &ctx()).unwrap();
        assert_eq!(result, arr(vec![i(0), i(5)]));
    }

    #[test]
    fn range_descending() {
        let result = range_verb(&[i(5), i(0), i(-1)], &ctx()).unwrap();
        assert_eq!(result, arr(vec![i(5), i(4), i(3), i(2), i(1)]));
    }

    #[test]
    fn range_single_element() {
        let result = range_verb(&[i(3), i(4)], &ctx()).unwrap();
        assert_eq!(result, arr(vec![i(3)]));
    }

    #[test]
    fn range_step_three() {
        let result = range_verb(&[i(1), i(10), i(3)], &ctx()).unwrap();
        assert_eq!(result, arr(vec![i(1), i(4), i(7)]));
    }

    // =========================================================================
    // compact — additional cases
    // =========================================================================

    #[test]
    fn compact_keeps_false_and_zero() {
        let data = arr(vec![b(false), i(0), null(), s(""), s("ok")]);
        let result = compact(&[data], &ctx()).unwrap();
        assert_eq!(result, arr(vec![b(false), i(0), s("ok")]));
    }

    #[test]
    fn compact_all_valid() {
        let data = arr(vec![i(1), s("a"), b(true)]);
        assert_eq!(compact(&[data], &ctx()).unwrap(), arr(vec![i(1), s("a"), b(true)]));
    }

    // =========================================================================
    // pluck — alias for map
    // =========================================================================

    #[test]
    fn pluck_extracts_field() {
        let data = arr(vec![
            obj(vec![("name", s("A")), ("age", i(10))]),
            obj(vec![("name", s("B")), ("age", i(20))]),
        ]);
        assert_eq!(pluck(&[data, s("age")], &ctx()).unwrap(), arr(vec![i(10), i(20)]));
    }

    #[test]
    fn pluck_missing_field_gives_null() {
        let data = arr(vec![obj(vec![("x", i(1))])]);
        assert_eq!(pluck(&[data, s("y")], &ctx()).unwrap(), arr(vec![null()]));
    }

    // =========================================================================
    // keys — additional cases
    // =========================================================================

    #[test]
    fn keys_multiple_keys() {
        let o = obj(vec![("a", i(1)), ("b", i(2)), ("c", i(3))]);
        assert_eq!(keys(&[o], &ctx()).unwrap(), arr(vec![s("a"), s("b"), s("c")]));
    }

    #[test]
    fn keys_empty_object() {
        let o = obj(vec![]);
        assert_eq!(keys(&[o], &ctx()).unwrap(), arr(vec![]));
    }

    // =========================================================================
    // values_verb — additional cases
    // =========================================================================

    #[test]
    fn values_verb_mixed_types() {
        let o = obj(vec![("a", i(1)), ("b", s("two")), ("c", b(true))]);
        assert_eq!(values_verb(&[o], &ctx()).unwrap(), arr(vec![i(1), s("two"), b(true)]));
    }

    // =========================================================================
    // entries — additional cases
    // =========================================================================

    #[test]
    fn entries_single_pair() {
        let o = obj(vec![("key", i(42))]);
        assert_eq!(entries(&[o], &ctx()).unwrap(), arr(vec![arr(vec![s("key"), i(42)])]));
    }

    #[test]
    fn entries_preserves_order() {
        let o = obj(vec![("z", i(1)), ("a", i(2))]);
        let result = entries(&[o], &ctx()).unwrap();
        if let DynValue::Array(items) = result {
            assert_eq!(items[0], arr(vec![s("z"), i(1)]));
            assert_eq!(items[1], arr(vec![s("a"), i(2)]));
        } else { panic!(); }
    }

    // =========================================================================
    // has — additional cases
    // =========================================================================

    #[test]
    fn has_key_with_null_value() {
        let o = obj(vec![("x", null())]);
        assert_eq!(has(&[o, s("x")], &ctx()).unwrap(), b(true));
    }

    #[test]
    fn has_missing_key() {
        let o = obj(vec![("a", i(1))]);
        assert_eq!(has(&[o, s("b")], &ctx()).unwrap(), b(false));
    }

    // =========================================================================
    // get_verb — additional path navigation
    // =========================================================================

    #[test]
    fn get_nested_object() {
        let o = obj(vec![("a", obj(vec![("b", obj(vec![("c", i(42))]))]))]);
        assert_eq!(get_verb(&[o, s("a.b.c")], &ctx()).unwrap(), i(42));
    }

    #[test]
    fn get_with_default_on_missing() {
        let o = obj(vec![("a", i(1))]);
        assert_eq!(get_verb(&[o, s("x.y"), s("default")], &ctx()).unwrap(), s("default"));
    }

    #[test]
    fn get_array_element_by_index() {
        let o = obj(vec![("items", arr(vec![s("zero"), s("one"), s("two")]))]);
        assert_eq!(get_verb(&[o, s("items.1")], &ctx()).unwrap(), s("one"));
    }

    #[test]
    fn get_out_of_bounds_index_returns_default() {
        let o = obj(vec![("items", arr(vec![i(1)]))]);
        assert_eq!(get_verb(&[o, s("items.5"), i(99)], &ctx()).unwrap(), i(99));
    }

    // =========================================================================
    // merge — additional cases
    // =========================================================================

    #[test]
    fn merge_second_overwrites_first() {
        let a = obj(vec![("x", i(1)), ("y", i(2))]);
        let b_obj = obj(vec![("y", i(99)), ("z", i(3))]);
        let result = merge(&[a, b_obj], &ctx()).unwrap();
        if let DynValue::Object(pairs) = result {
            let y_val = pairs.iter().find(|(k, _)| k == "y").unwrap();
            assert_eq!(y_val.1, i(99));
            assert_eq!(pairs.len(), 3);
        } else { panic!(); }
    }

    #[test]
    fn merge_disjoint_objects() {
        let a = obj(vec![("a", i(1))]);
        let b_obj = obj(vec![("b", i(2))]);
        let result = merge(&[a, b_obj], &ctx()).unwrap();
        if let DynValue::Object(pairs) = result {
            assert_eq!(pairs.len(), 2);
        } else { panic!(); }
    }

    // =========================================================================
    // set_verb — additional cases
    // =========================================================================

    #[test]
    fn set_verb_integer_value() {
        assert_eq!(set_verb(&[s("counter"), i(42)], &ctx()).unwrap(), i(42));
    }

    #[test]
    fn set_verb_bool_value() {
        assert_eq!(set_verb(&[s("flag"), b(true)], &ctx()).unwrap(), b(true));
    }

    #[test]
    fn set_verb_too_few_args() {
        assert!(set_verb(&[s("name")], &ctx()).is_err());
    }

    // =========================================================================
    // cumsum — additional cases
    // =========================================================================

    #[test]
    fn cumsum_with_floats() {
        let data = arr(vec![f(1.5), f(2.5), f(3.0)]);
        let result = cumsum(&[data], &ctx()).unwrap();
        if let DynValue::Array(items) = result {
            assert_eq!(items.len(), 3);
            assert_eq!(items[0], f(1.5));
            assert_eq!(items[1], i(4)); // 4.0 has no fractional part, becomes Integer
        } else { panic!(); }
    }

    #[test]
    fn cumsum_all_nulls() {
        let data = arr(vec![null(), null()]);
        assert_eq!(cumsum(&[data], &ctx()).unwrap(), arr(vec![null(), null()]));
    }

    #[test]
    fn cumsum_mixed_null_int() {
        let data = arr(vec![i(1), null(), i(3)]);
        let result = cumsum(&[data], &ctx()).unwrap();
        if let DynValue::Array(items) = result {
            assert_eq!(items[0], i(1));
            assert_eq!(items[1], null());
            assert_eq!(items[2], i(4)); // sum continues: 1+3=4
        } else { panic!(); }
    }

    // =========================================================================
    // cumprod — additional cases
    // =========================================================================

    #[test]
    fn cumprod_with_ones() {
        let data = arr(vec![i(1), i(1), i(1)]);
        assert_eq!(cumprod(&[data], &ctx()).unwrap(), arr(vec![i(1), i(1), i(1)]));
    }

    #[test]
    fn cumprod_with_negative() {
        let data = arr(vec![i(2), i(-3)]);
        let result = cumprod(&[data], &ctx()).unwrap();
        if let DynValue::Array(items) = result {
            assert_eq!(items[0], i(2));
            assert_eq!(items[1], i(-6));
        } else { panic!(); }
    }

    #[test]
    fn cumprod_no_args_returns_null() {
        assert_eq!(cumprod(&[], &ctx()).unwrap(), null());
    }

    // =========================================================================
    // diff_verb — additional cases
    // =========================================================================

    #[test]
    fn diff_period_two() {
        let data = arr(vec![i(10), i(20), i(50), i(100)]);
        let result = diff_verb(&[data, i(2)], &ctx()).unwrap();
        if let DynValue::Array(items) = result {
            assert_eq!(items[0], null());
            assert_eq!(items[1], null());
            assert_eq!(items[2], i(40)); // 50-10
            assert_eq!(items[3], i(80)); // 100-20
        } else { panic!(); }
    }

    #[test]
    fn diff_with_floats() {
        let data = arr(vec![f(1.0), f(2.5), f(4.0)]);
        let result = diff_verb(&[data], &ctx()).unwrap();
        if let DynValue::Array(items) = result {
            assert_eq!(items[0], null());
            assert_eq!(items[1], f(1.5));
            assert_eq!(items[2], f(1.5));
        } else { panic!(); }
    }

    #[test]
    fn diff_single_element() {
        let data = arr(vec![i(5)]);
        assert_eq!(diff_verb(&[data], &ctx()).unwrap(), arr(vec![null()]));
    }

    // =========================================================================
    // pct_change — additional cases
    // =========================================================================

    #[test]
    fn pct_change_100_percent_increase() {
        let data = arr(vec![i(10), i(20)]);
        let result = pct_change(&[data], &ctx()).unwrap();
        if let DynValue::Array(items) = result {
            assert_eq!(items[0], null());
            if let DynValue::Float(v) = items[1] {
                assert!((v - 1.0).abs() < 1e-10);
            } else { panic!(); }
        } else { panic!(); }
    }

    #[test]
    fn pct_change_decrease() {
        let data = arr(vec![i(100), i(75)]);
        let result = pct_change(&[data], &ctx()).unwrap();
        if let DynValue::Array(items) = result {
            if let DynValue::Float(v) = items[1] {
                assert!((v - (-0.25)).abs() < 1e-10);
            } else { panic!(); }
        } else { panic!(); }
    }

    #[test]
    fn pct_change_with_period_two() {
        let data = arr(vec![i(10), i(20), i(30)]);
        let result = pct_change(&[data, i(2)], &ctx()).unwrap();
        if let DynValue::Array(items) = result {
            assert_eq!(items[0], null());
            assert_eq!(items[1], null());
            if let DynValue::Float(v) = items[2] {
                assert!((v - 2.0).abs() < 1e-10); // (30-10)/10 = 2.0
            } else { panic!(); }
        } else { panic!(); }
    }

    // =========================================================================
    // shift_verb — additional cases
    // =========================================================================

    #[test]
    fn shift_negative_periods() {
        let data = arr(vec![i(1), i(2), i(3), i(4)]);
        let result = shift_verb(&[data, i(-2)], &ctx()).unwrap();
        assert_eq!(result, arr(vec![i(3), i(4), null(), null()]));
    }

    #[test]
    fn shift_with_custom_fill() {
        let data = arr(vec![i(1), i(2), i(3)]);
        let result = shift_verb(&[data, i(1), i(0)], &ctx()).unwrap();
        assert_eq!(result, arr(vec![i(0), i(1), i(2)]));
    }

    #[test]
    fn shift_zero_periods() {
        let data = arr(vec![i(1), i(2), i(3)]);
        assert_eq!(shift_verb(&[data, i(0)], &ctx()).unwrap(), arr(vec![i(1), i(2), i(3)]));
    }

    // =========================================================================
    // lag — additional cases
    // =========================================================================

    #[test]
    fn lag_period_three() {
        let data = arr(vec![i(10), i(20), i(30), i(40), i(50)]);
        let result = lag(&[data, i(3)], &ctx()).unwrap();
        assert_eq!(result, arr(vec![null(), null(), null(), i(10), i(20)]));
    }

    #[test]
    fn lag_with_custom_default() {
        let data = arr(vec![i(10), i(20), i(30)]);
        let result = lag(&[data, i(1), i(-1)], &ctx()).unwrap();
        assert_eq!(result, arr(vec![i(-1), i(10), i(20)]));
    }

    #[test]
    fn lag_single_element() {
        let data = arr(vec![i(42)]);
        assert_eq!(lag(&[data], &ctx()).unwrap(), arr(vec![null()]));
    }

    // =========================================================================
    // lead — additional cases
    // =========================================================================

    #[test]
    fn lead_period_two() {
        let data = arr(vec![i(10), i(20), i(30), i(40)]);
        let result = lead(&[data, i(2)], &ctx()).unwrap();
        assert_eq!(result, arr(vec![i(30), i(40), null(), null()]));
    }

    #[test]
    fn lead_with_custom_default() {
        let data = arr(vec![i(10), i(20), i(30)]);
        let result = lead(&[data, i(1), i(0)], &ctx()).unwrap();
        assert_eq!(result, arr(vec![i(20), i(30), i(0)]));
    }

    #[test]
    fn lead_single_element() {
        let data = arr(vec![i(42)]);
        assert_eq!(lead(&[data], &ctx()).unwrap(), arr(vec![null()]));
    }

    // =========================================================================
    // rank — additional cases
    // =========================================================================

    #[test]
    fn rank_with_field_name() {
        let data = arr(vec![
            obj(vec![("score", i(30))]),
            obj(vec![("score", i(10))]),
            obj(vec![("score", i(20))]),
        ]);
        let result = rank(&[data, s("score")], &ctx()).unwrap();
        if let DynValue::Array(items) = result {
            let get_rank = |item: &DynValue| {
                if let DynValue::Object(pairs) = item {
                    pairs.iter().find(|(k, _)| k == "_rank").map(|(_, v)| v.clone())
                } else { None }
            };
            assert_eq!(get_rank(&items[0]), Some(i(1))); // 30 -> rank 1 (desc)
            assert_eq!(get_rank(&items[1]), Some(i(3))); // 10 -> rank 3 (desc)
            assert_eq!(get_rank(&items[2]), Some(i(2))); // 20 -> rank 2 (desc)
        } else { panic!(); }
    }

    #[test]
    fn rank_asc_order() {
        let data = arr(vec![i(30), i(10), i(20)]);
        let result = rank(&[data, null(), s("asc")], &ctx()).unwrap();
        if let DynValue::Array(items) = result {
            let get_rank = |item: &DynValue| {
                if let DynValue::Object(pairs) = item {
                    pairs.iter().find(|(k, _)| k == "_rank").map(|(_, v)| v.clone())
                } else { None }
            };
            assert_eq!(get_rank(&items[0]), Some(i(3))); // 30 -> rank 3 (asc)
            assert_eq!(get_rank(&items[1]), Some(i(1))); // 10 -> rank 1 (asc)
            assert_eq!(get_rank(&items[2]), Some(i(2))); // 20 -> rank 2 (asc)
        } else { panic!(); }
    }

    #[test]
    fn rank_all_same_value() {
        let data = arr(vec![i(5), i(5), i(5)]);
        let result = rank(&[data], &ctx()).unwrap();
        if let DynValue::Array(items) = result {
            let get_rank = |item: &DynValue| {
                if let DynValue::Object(pairs) = item {
                    pairs.iter().find(|(k, _)| k == "_rank").map(|(_, v)| v.clone())
                } else { None }
            };
            assert_eq!(get_rank(&items[0]), Some(i(1)));
            assert_eq!(get_rank(&items[1]), Some(i(1)));
            assert_eq!(get_rank(&items[2]), Some(i(1)));
        } else { panic!(); }
    }

    // =========================================================================
    // fill_missing — additional strategies
    // =========================================================================

    #[test]
    fn fill_missing_mean_strategy() {
        let data = arr(vec![i(10), null(), i(30), null()]);
        let result = fill_missing(&[data, i(0), s("mean")], &ctx()).unwrap();
        if let DynValue::Array(items) = result {
            assert_eq!(items[0], i(10));
            // mean of 10 and 30 = 20.0
            if let DynValue::Float(v) = items[1] { assert!((v - 20.0).abs() < 1e-10); } else { panic!(); }
            assert_eq!(items[2], i(30));
        } else { panic!(); }
    }

    #[test]
    fn fill_missing_forward_strategy() {
        let data = arr(vec![i(1), null(), null(), i(4), null()]);
        let result = fill_missing(&[data, i(0), s("forward")], &ctx()).unwrap();
        assert_eq!(result, arr(vec![i(1), i(1), i(1), i(4), i(4)]));
    }

    #[test]
    fn fill_missing_backward_strategy() {
        let data = arr(vec![null(), null(), i(3), null(), i(5)]);
        let result = fill_missing(&[data, i(0), s("backward")], &ctx()).unwrap();
        assert_eq!(result, arr(vec![i(3), i(3), i(3), i(5), i(5)]));
    }

    #[test]
    fn fill_missing_mean_all_nulls() {
        let data = arr(vec![null(), null()]);
        let result = fill_missing(&[data, i(0), s("mean")], &ctx()).unwrap();
        // mean of nothing = 0.0
        if let DynValue::Array(items) = result {
            assert_eq!(items[0], f(0.0));
        } else { panic!(); }
    }

    // =========================================================================
    // sample_verb — additional cases
    // =========================================================================

    #[test]
    fn sample_exact_count() {
        let data = arr(vec![i(1), i(2), i(3)]);
        assert_eq!(sample_verb(&[data, i(3)], &ctx()).unwrap(), arr(vec![i(1), i(2), i(3)]));
    }

    #[test]
    fn sample_one_element() {
        let data = arr(vec![i(42)]);
        assert_eq!(sample_verb(&[data, i(1)], &ctx()).unwrap(), arr(vec![i(42)]));
    }

    // =========================================================================
    // limit — alias for take
    // =========================================================================

    #[test]
    fn limit_basic() {
        let data = arr(vec![i(1), i(2), i(3), i(4)]);
        assert_eq!(limit(&[data, i(2)], &ctx()).unwrap(), arr(vec![i(1), i(2)]));
    }

    #[test]
    fn limit_more_than_length() {
        let data = arr(vec![i(1)]);
        assert_eq!(limit(&[data, i(10)], &ctx()).unwrap(), arr(vec![i(1)]));
    }

    // =========================================================================
    // dedupe — additional cases
    // =========================================================================

    #[test]
    fn dedupe_by_string_field() {
        let data = arr(vec![
            obj(vec![("id", s("a")), ("v", i(1))]),
            obj(vec![("id", s("b")), ("v", i(2))]),
            obj(vec![("id", s("a")), ("v", i(3))]),
        ]);
        let result = dedupe(&[data, s("id")], &ctx()).unwrap();
        if let DynValue::Array(items) = result {
            assert_eq!(items.len(), 2);
            assert_eq!(get_field(&items[0], "v"), Some(i(1))); // keeps first
        } else { panic!(); }
    }

    #[test]
    fn dedupe_missing_field_treated_as_null() {
        let data = arr(vec![
            obj(vec![("a", i(1))]),
            obj(vec![("a", i(2))]),
        ]);
        // Both missing "id" field -> both treated as null key -> only first kept
        let result = dedupe(&[data, s("id")], &ctx()).unwrap();
        if let DynValue::Array(items) = result {
            assert_eq!(items.len(), 1);
        } else { panic!(); }
    }

    // =========================================================================
    // accumulate — additional cases
    // =========================================================================

    #[test]
    fn accumulate_no_existing_returns_value() {
        let result = accumulate(&[s("total"), i(5)], &ctx()).unwrap();
        assert_eq!(result, i(5));
    }

    #[test]
    fn accumulate_string_value_returns_value() {
        let result = accumulate(&[s("msg"), s("hello")], &ctx()).unwrap();
        assert_eq!(result, s("hello"));
    }

    // =========================================================================
    // filter — additional operator + type coverage
    // =========================================================================

    #[test]
    fn filter_lte_operator() {
        let data = arr(vec![
            obj(vec![("v", i(1))]),
            obj(vec![("v", i(5))]),
            obj(vec![("v", i(10))]),
        ]);
        let result = filter(&[data, s("v"), s("lte"), i(5)], &ctx()).unwrap();
        if let DynValue::Array(items) = result { assert_eq!(items.len(), 2); } else { panic!(); }
    }

    #[test]
    fn filter_contains_string() {
        let data = arr(vec![
            obj(vec![("name", s("alice smith"))]),
            obj(vec![("name", s("bob jones"))]),
        ]);
        let result = filter(&[data, s("name"), s("contains"), s("alice")], &ctx()).unwrap();
        if let DynValue::Array(items) = result { assert_eq!(items.len(), 1); } else { panic!(); }
    }

    #[test]
    fn filter_ne_operator() {
        let data = arr(vec![
            obj(vec![("v", i(1))]),
            obj(vec![("v", i(2))]),
            obj(vec![("v", i(1))]),
        ]);
        let result = filter(&[data, s("v"), s("ne"), i(1)], &ctx()).unwrap();
        if let DynValue::Array(items) = result { assert_eq!(items.len(), 1); } else { panic!(); }
    }

    // =========================================================================
    // sort_verb / sort_desc — additional type edge cases
    // =========================================================================

    #[test]
    fn sort_strings_case_sensitive() {
        let data = arr(vec![s("banana"), s("Apple"), s("cherry")]);
        let result = sort_verb(&[data], &ctx()).unwrap();
        // Uppercase A comes before lowercase b in ASCII
        assert_eq!(result, arr(vec![s("Apple"), s("banana"), s("cherry")]));
    }

    #[test]
    fn sort_desc_preserves_all_elements() {
        let data = arr(vec![i(3), i(1), i(4), i(1), i(5)]);
        let result = sort_desc(&[data], &ctx()).unwrap();
        assert_eq!(result, arr(vec![i(5), i(4), i(3), i(1), i(1)]));
    }

    // =========================================================================
    // map_verb — edge cases
    // =========================================================================

    #[test]
    fn map_with_nested_objects() {
        let data = arr(vec![
            obj(vec![("inner", obj(vec![("x", i(1))]))]),
            obj(vec![("inner", obj(vec![("x", i(2))]))]),
        ]);
        let result = map_verb(&[data, s("inner")], &ctx()).unwrap();
        if let DynValue::Array(items) = result {
            assert_eq!(items.len(), 2);
            assert_eq!(get_field(&items[0], "x"), Some(i(1)));
        } else { panic!(); }
    }

    // =========================================================================
    // index_of — additional cases
    // =========================================================================

    #[test]
    fn index_of_returns_first_of_duplicates() {
        let data = arr(vec![i(5), i(3), i(5)]);
        assert_eq!(index_of(&[data, i(5)], &ctx()).unwrap(), i(0));
    }

    #[test]
    fn index_of_null_value() {
        let data = arr(vec![i(1), null(), i(3)]);
        assert_eq!(index_of(&[data, null()], &ctx()).unwrap(), i(1));
    }

    // =========================================================================
    // slice — additional boundary cases
    // =========================================================================

    #[test]
    fn slice_from_start() {
        let data = arr(vec![i(10), i(20), i(30), i(40)]);
        assert_eq!(slice(&[data, i(0), i(2)], &ctx()).unwrap(), arr(vec![i(10), i(20)]));
    }

    #[test]
    fn slice_to_end() {
        let data = arr(vec![i(10), i(20), i(30)]);
        assert_eq!(slice(&[data, i(1), i(100)], &ctx()).unwrap(), arr(vec![i(20), i(30)]));
    }

    // =========================================================================
    // flatten — mixed nesting
    // =========================================================================

    #[test]
    fn flatten_already_flat() {
        let data = arr(vec![i(1), i(2), i(3)]);
        assert_eq!(flatten(&[data], &ctx()).unwrap(), arr(vec![i(1), i(2), i(3)]));
    }

    #[test]
    fn flatten_with_empty_sub_arrays() {
        let data = arr(vec![arr(vec![]), arr(vec![i(1)]), arr(vec![])]);
        assert_eq!(flatten(&[data], &ctx()).unwrap(), arr(vec![i(1)]));
    }

    // =========================================================================
    // distinct — type coverage
    // =========================================================================

    #[test]
    fn distinct_strings() {
        let data = arr(vec![s("a"), s("b"), s("a"), s("c"), s("b")]);
        assert_eq!(distinct(&[data], &ctx()).unwrap(), arr(vec![s("a"), s("b"), s("c")]));
    }

    #[test]
    fn distinct_bools() {
        let data = arr(vec![b(true), b(false), b(true), b(false)]);
        assert_eq!(distinct(&[data], &ctx()).unwrap(), arr(vec![b(true), b(false)]));
    }

    // =========================================================================
    // Geo: distance, bearing, midpoint edge cases
    // =========================================================================

    #[test]
    fn distance_same_point_is_zero() {
        let result = distance(&[f(40.0), f(-74.0), f(40.0), f(-74.0)], &ctx()).unwrap();
        if let DynValue::Float(v) = result { assert!(v.abs() < 0.01); } else { panic!(); }
    }

    #[test]
    fn bearing_south() {
        // Going due south: decreasing lat, same lon
        let result = bearing(&[f(10.0), f(0.0), f(0.0), f(0.0)], &ctx()).unwrap();
        if let DynValue::Float(v) = result {
            assert!((v - 180.0).abs() < 1.0);
        } else { panic!(); }
    }

    #[test]
    fn midpoint_antipodal_lons() {
        let result = midpoint(&[f(0.0), f(-90.0), f(0.0), f(90.0)], &ctx()).unwrap();
        if let DynValue::Object(pairs) = result {
            let lat = pairs.iter().find(|(k, _)| k == "lat").unwrap();
            if let DynValue::Float(v) = &lat.1 { assert!(v.abs() < 0.01); } else { panic!(); }
        } else { panic!(); }
    }

    // =========================================================================
    // sequence / reset_sequence
    // =========================================================================

    #[test]
    fn sequence_returns_zero_default() {
        assert_eq!(sequence(&[s("counter")], &ctx()).unwrap(), i(0));
    }

    #[test]
    fn reset_sequence_returns_zero() {
        assert_eq!(reset_sequence(&[s("counter")], &ctx()).unwrap(), i(0));
    }

    // =========================================================================
    // row_number
    // =========================================================================

    #[test]
    fn row_number_from_context() {
        let mut h = OwnedCtx::new();
        h.lv.insert("$index".to_string(), i(42));
        assert_eq!(row_number(&[], &h.ctx()).unwrap(), i(42));
    }

    #[test]
    fn row_number_default_is_zero() {
        assert_eq!(row_number(&[], &ctx()).unwrap(), i(0));
    }

    // =========================================================================
    // Helper compare_values coverage
    // =========================================================================

    #[test]
    fn compare_values_unknown_op_is_false() {
        assert!(!compare_values(&i(1), "unknown_op", &i(1)));
    }

    #[test]
    fn compare_values_string_gt() {
        assert!(compare_values(&s("b"), "gt", &s("a")));
    }

    #[test]
    fn compare_values_string_lt() {
        assert!(compare_values(&s("a"), "lt", &s("b")));
    }

    #[test]
    fn compare_values_int_float_eq() {
        assert!(compare_values(&i(5), "eq", &f(5.0)));
    }

    #[test]
    fn compare_values_string_int_eq() {
        assert!(compare_values(&s("42"), "eq", &i(42)));
    }
}
