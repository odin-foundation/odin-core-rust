//! Verb implementations for the transform engine.
//!
//! Organized into 13 categories matching the TypeScript reference:
//! - core: concat, upper, lower, trim, coalesce, ifNull, ifEmpty, ifElse, lookup
//! - coercion: coerceString, coerceNumber, coerceBoolean, coerceDate, tryCoerce, toArray, toObject
//! - logic: and, or, not, eq, ne, lt, lte, gt, gte, between, isNull, isString, typeOf, cond
//! - string: capitalize, titleCase, replace, split, case transforms, fuzzy matching
//! - numeric: formatNumber, abs, round, add, divide, mod, sign, random, safeDivide
//! - datetime: formatDate, parseDate, addDays, addMonths, dateDiff, dayOfWeek, quarter
//! - array: filter, map, sort, flatten, distinct, zip, groupBy, partition, chunk
//! - encoding: base64Encode/Decode, urlEncode/Decode, hash functions
//! - financial: compound, pmt, fv, pv, npv, irr, std, variance, median
//! - object: keys, values, entries, has, get, merge
//! - aggregation: sum, count, min, max, avg, first, last, accumulate
//! - generation: uuid, sequence, resetSequence
//! - geo: distance, inBoundingBox, bearing, midpoint

mod string_verbs;
mod collection_verbs;
mod numeric_verbs;

use std::collections::HashMap;
use std::sync::OnceLock;
use crate::types::transform::DynValue;

/// A verb function signature.
pub type VerbFn = fn(&[DynValue], &VerbContext) -> Result<DynValue, String>;

/// Context available to verbs during execution.
pub struct VerbContext {
    /// Source data for the current record.
    pub source: DynValue,
    /// Loop variables (current item, index, etc.).
    pub loop_vars: HashMap<String, DynValue>,
    /// Accumulators for aggregation.
    pub accumulators: HashMap<String, DynValue>,
    /// Lookup tables.
    pub tables: HashMap<String, crate::types::transform::LookupTable>,
}

/// Global singleton for built-in verbs (initialized once, shared across all executions).
static BUILTIN_VERBS: OnceLock<HashMap<String, VerbFn>> = OnceLock::new();

fn get_builtins() -> &'static HashMap<String, VerbFn> {
    BUILTIN_VERBS.get_or_init(|| {
        let mut m = HashMap::with_capacity(256);
        register_builtins(&mut m);
        m
    })
}

/// Registry of all available verbs.
pub struct VerbRegistry {
    custom: HashMap<String, VerbFn>,
}

impl Default for VerbRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl VerbRegistry {
    /// Create a new registry. Built-in verbs come from a global singleton.
    pub fn new() -> Self {
        Self {
            custom: HashMap::new(),
        }
    }

    /// Look up a verb by name.
    pub fn get(&self, name: &str) -> Option<&VerbFn> {
        self.custom.get(name).or_else(|| get_builtins().get(name))
    }

    /// Register a custom verb.
    pub fn register_custom(&mut self, name: String, func: VerbFn) {
        self.custom.insert(name, func);
    }
}

/// Register all built-in verbs into the given map.
fn register_builtins(builtins: &mut HashMap<String, VerbFn>) {
        // Core verbs
        builtins.insert("concat".to_string(), verb_concat);
        builtins.insert("upper".to_string(), verb_upper);
        builtins.insert("lower".to_string(), verb_lower);
        builtins.insert("trim".to_string(), verb_trim);
        builtins.insert("coalesce".to_string(), verb_coalesce);

        // Type coercion
        builtins.insert("coerceString".to_string(), verb_coerce_string);
        builtins.insert("coerceNumber".to_string(), verb_coerce_number);
        builtins.insert("coerceBoolean".to_string(), verb_coerce_boolean);

        // Logic
        builtins.insert("eq".to_string(), verb_eq);
        builtins.insert("ne".to_string(), verb_ne);
        builtins.insert("not".to_string(), verb_not);
        builtins.insert("isNull".to_string(), verb_is_null);

        // String
        builtins.insert("capitalize".to_string(), verb_capitalize);
        builtins.insert("replace".to_string(), verb_replace);
        builtins.insert("substring".to_string(), verb_substring);
        builtins.insert("length".to_string(), verb_length);

        // Numeric
        builtins.insert("add".to_string(), verb_add);
        builtins.insert("subtract".to_string(), verb_subtract);
        builtins.insert("multiply".to_string(), verb_multiply);
        builtins.insert("divide".to_string(), verb_divide);
        builtins.insert("abs".to_string(), verb_abs);
        builtins.insert("round".to_string(), verb_round);

        // Core (additional)
        builtins.insert("trimLeft".to_string(), verb_trim_left);
        builtins.insert("trimRight".to_string(), verb_trim_right);
        builtins.insert("ifNull".to_string(), verb_if_null);
        builtins.insert("ifEmpty".to_string(), verb_if_empty);
        builtins.insert("ifElse".to_string(), verb_if_else);
        builtins.insert("lookup".to_string(), verb_lookup);
        builtins.insert("lookupDefault".to_string(), verb_lookup_default);


        // Logic (additional)
        builtins.insert("and".to_string(), verb_and);
        builtins.insert("or".to_string(), verb_or);
        builtins.insert("xor".to_string(), verb_xor);
        builtins.insert("lt".to_string(), verb_lt);
        builtins.insert("lte".to_string(), verb_lte);
        builtins.insert("gt".to_string(), verb_gt);
        builtins.insert("gte".to_string(), verb_gte);
        builtins.insert("between".to_string(), verb_between);
        builtins.insert("isString".to_string(), verb_is_string);
        builtins.insert("isNumber".to_string(), verb_is_number);
        builtins.insert("isBoolean".to_string(), verb_is_boolean);
        builtins.insert("isArray".to_string(), verb_is_array);
        builtins.insert("isObject".to_string(), verb_is_object);
        builtins.insert("isDate".to_string(), verb_is_date);
        builtins.insert("typeOf".to_string(), verb_type_of);
        builtins.insert("cond".to_string(), verb_cond);
        builtins.insert("assert".to_string(), verb_assert);

        // Coercion (additional)
        builtins.insert("coerceInteger".to_string(), verb_coerce_integer);
        builtins.insert("coerceDate".to_string(), verb_coerce_date);
        builtins.insert("coerceTimestamp".to_string(), verb_coerce_timestamp);
        builtins.insert("tryCoerce".to_string(), verb_try_coerce);
        builtins.insert("toArray".to_string(), verb_to_array);
        builtins.insert("toObject".to_string(), verb_to_object);

        // String verbs (30)
        builtins.insert("titleCase".to_string(), string_verbs::verb_title_case);
        builtins.insert("contains".to_string(), string_verbs::verb_contains);
        builtins.insert("startsWith".to_string(), string_verbs::verb_starts_with);
        builtins.insert("endsWith".to_string(), string_verbs::verb_ends_with);
        builtins.insert("replaceRegex".to_string(), string_verbs::verb_replace_regex);
        builtins.insert("padLeft".to_string(), string_verbs::verb_pad_left);
        builtins.insert("padRight".to_string(), string_verbs::verb_pad_right);
        builtins.insert("pad".to_string(), string_verbs::verb_pad);
        builtins.insert("truncate".to_string(), string_verbs::verb_truncate);
        builtins.insert("split".to_string(), string_verbs::verb_split);
        builtins.insert("join".to_string(), string_verbs::verb_join);
        builtins.insert("mask".to_string(), string_verbs::verb_mask);
        builtins.insert("reverseString".to_string(), string_verbs::verb_reverse_string);
        builtins.insert("repeat".to_string(), string_verbs::verb_repeat);
        builtins.insert("camelCase".to_string(), string_verbs::verb_camel_case);
        builtins.insert("snakeCase".to_string(), string_verbs::verb_snake_case);
        builtins.insert("kebabCase".to_string(), string_verbs::verb_kebab_case);
        builtins.insert("pascalCase".to_string(), string_verbs::verb_pascal_case);
        builtins.insert("slugify".to_string(), string_verbs::verb_slugify);
        builtins.insert("match".to_string(), string_verbs::verb_match);
        builtins.insert("extract".to_string(), string_verbs::verb_extract);
        builtins.insert("normalizeSpace".to_string(), string_verbs::verb_normalize_space);
        builtins.insert("leftOf".to_string(), string_verbs::verb_left_of);
        builtins.insert("rightOf".to_string(), string_verbs::verb_right_of);
        builtins.insert("wrap".to_string(), string_verbs::verb_wrap);
        builtins.insert("center".to_string(), string_verbs::verb_center);
        builtins.insert("matches".to_string(), string_verbs::verb_matches);
        builtins.insert("stripAccents".to_string(), string_verbs::verb_strip_accents);
        builtins.insert("clean".to_string(), string_verbs::verb_clean);
        builtins.insert("wordCount".to_string(), string_verbs::verb_word_count);

        // Text analysis verbs
        builtins.insert("tokenize".to_string(), string_verbs::verb_tokenize);
        builtins.insert("levenshtein".to_string(), string_verbs::verb_levenshtein);
        builtins.insert("soundex".to_string(), string_verbs::verb_soundex);

        // Encoding verbs (11)
        builtins.insert("base64Encode".to_string(), string_verbs::verb_base64_encode);
        builtins.insert("base64Decode".to_string(), string_verbs::verb_base64_decode);
        builtins.insert("urlEncode".to_string(), string_verbs::verb_url_encode);
        builtins.insert("urlDecode".to_string(), string_verbs::verb_url_decode);
        builtins.insert("jsonEncode".to_string(), string_verbs::verb_json_encode);
        builtins.insert("jsonDecode".to_string(), string_verbs::verb_json_decode);
        builtins.insert("hexEncode".to_string(), string_verbs::verb_hex_encode);
        builtins.insert("hexDecode".to_string(), string_verbs::verb_hex_decode);
        builtins.insert("sha256".to_string(), string_verbs::verb_sha256);
        builtins.insert("md5".to_string(), string_verbs::verb_md5);
        builtins.insert("crc32".to_string(), string_verbs::verb_crc32);

        // Array verbs (30)
        builtins.insert("filter".to_string(), collection_verbs::filter);
        builtins.insert("flatten".to_string(), collection_verbs::flatten);
        builtins.insert("distinct".to_string(), collection_verbs::distinct);
        builtins.insert("unique".to_string(), collection_verbs::unique);
        builtins.insert("sort".to_string(), collection_verbs::sort_verb);
        builtins.insert("sortDesc".to_string(), collection_verbs::sort_desc);
        builtins.insert("sortBy".to_string(), collection_verbs::sort_by);
        builtins.insert("map".to_string(), collection_verbs::map_verb);
        builtins.insert("indexOf".to_string(), collection_verbs::index_of);
        builtins.insert("at".to_string(), collection_verbs::at);
        builtins.insert("slice".to_string(), collection_verbs::slice);
        builtins.insert("reverse".to_string(), collection_verbs::reverse);
        builtins.insert("every".to_string(), collection_verbs::every);
        builtins.insert("some".to_string(), collection_verbs::some);
        builtins.insert("find".to_string(), collection_verbs::find);
        builtins.insert("findIndex".to_string(), collection_verbs::find_index);
        builtins.insert("includes".to_string(), collection_verbs::includes);
        builtins.insert("concatArrays".to_string(), collection_verbs::concat_arrays);
        builtins.insert("zip".to_string(), collection_verbs::zip_verb);
        builtins.insert("groupBy".to_string(), collection_verbs::group_by);
        builtins.insert("partition".to_string(), collection_verbs::partition);
        builtins.insert("take".to_string(), collection_verbs::take);
        builtins.insert("drop".to_string(), collection_verbs::drop);
        builtins.insert("chunk".to_string(), collection_verbs::chunk);
        builtins.insert("range".to_string(), collection_verbs::range_verb);
        builtins.insert("compact".to_string(), collection_verbs::compact);
        builtins.insert("pluck".to_string(), collection_verbs::pluck);
        builtins.insert("rowNumber".to_string(), collection_verbs::row_number);
        builtins.insert("sample".to_string(), collection_verbs::sample_verb);
        builtins.insert("limit".to_string(), collection_verbs::limit);
        builtins.insert("dedupe".to_string(), collection_verbs::dedupe);

        // Object verbs (6)
        builtins.insert("keys".to_string(), collection_verbs::keys);
        builtins.insert("values".to_string(), collection_verbs::values_verb);
        builtins.insert("entries".to_string(), collection_verbs::entries);
        builtins.insert("has".to_string(), collection_verbs::has);
        builtins.insert("get".to_string(), collection_verbs::get_verb);
        builtins.insert("merge".to_string(), collection_verbs::merge);

        // Aggregation verbs (9)
        builtins.insert("accumulate".to_string(), collection_verbs::accumulate);
        builtins.insert("set".to_string(), collection_verbs::set_verb);
        builtins.insert("sum".to_string(), collection_verbs::sum_verb);
        builtins.insert("count".to_string(), collection_verbs::count_verb);
        builtins.insert("min".to_string(), collection_verbs::min_verb);
        builtins.insert("max".to_string(), collection_verbs::max_verb);
        builtins.insert("avg".to_string(), collection_verbs::avg);
        builtins.insert("first".to_string(), collection_verbs::first_verb);
        builtins.insert("last".to_string(), collection_verbs::last_verb);

        // Generation verbs (3)
        builtins.insert("uuid".to_string(), collection_verbs::uuid_verb);
        builtins.insert("sequence".to_string(), collection_verbs::sequence);
        builtins.insert("resetSequence".to_string(), collection_verbs::reset_sequence);

        // Geo verbs (6)
        builtins.insert("distance".to_string(), collection_verbs::distance);
        builtins.insert("inBoundingBox".to_string(), collection_verbs::in_bounding_box);
        builtins.insert("toRadians".to_string(), collection_verbs::to_radians);
        builtins.insert("toDegrees".to_string(), collection_verbs::to_degrees);
        builtins.insert("bearing".to_string(), collection_verbs::bearing);
        builtins.insert("midpoint".to_string(), collection_verbs::midpoint);

        // Timeseries verbs
        builtins.insert("cumsum".to_string(), collection_verbs::cumsum);
        builtins.insert("cumprod".to_string(), collection_verbs::cumprod);
        builtins.insert("diff".to_string(), collection_verbs::diff_verb);
        builtins.insert("pctChange".to_string(), collection_verbs::pct_change);
        builtins.insert("shift".to_string(), collection_verbs::shift_verb);

        // Numeric verbs (19 — add/subtract/multiply/divide/abs/round already registered above)
        builtins.insert("formatNumber".to_string(), numeric_verbs::format_number);
        builtins.insert("formatInteger".to_string(), numeric_verbs::format_integer);
        builtins.insert("formatCurrency".to_string(), numeric_verbs::format_currency);
        builtins.insert("floor".to_string(), numeric_verbs::floor);
        builtins.insert("ceil".to_string(), numeric_verbs::ceil);

        builtins.insert("negate".to_string(), numeric_verbs::negate);
        builtins.insert("switch".to_string(), numeric_verbs::switch_verb);
        builtins.insert("sign".to_string(), numeric_verbs::sign);
        builtins.insert("trunc".to_string(), numeric_verbs::trunc);
        builtins.insert("random".to_string(), numeric_verbs::random_verb);
        builtins.insert("minOf".to_string(), numeric_verbs::min_of);
        builtins.insert("maxOf".to_string(), numeric_verbs::max_of);
        builtins.insert("formatPercent".to_string(), numeric_verbs::format_percent);
        builtins.insert("isFinite".to_string(), numeric_verbs::is_finite);
        builtins.insert("isNaN".to_string(), numeric_verbs::is_nan);
        builtins.insert("parseInt".to_string(), numeric_verbs::parse_int);
        builtins.insert("safeDivide".to_string(), numeric_verbs::safe_divide);
        builtins.insert("formatLocaleNumber".to_string(), numeric_verbs::format_locale_number);

        // DateTime verbs (26)
        builtins.insert("today".to_string(), numeric_verbs::today);
        builtins.insert("now".to_string(), numeric_verbs::now_verb);
        builtins.insert("formatDate".to_string(), numeric_verbs::format_date);
        builtins.insert("parseDate".to_string(), numeric_verbs::parse_date);
        builtins.insert("formatTime".to_string(), numeric_verbs::format_time);
        builtins.insert("formatTimestamp".to_string(), numeric_verbs::format_timestamp_verb);
        builtins.insert("parseTimestamp".to_string(), numeric_verbs::parse_timestamp_verb);
        builtins.insert("addDays".to_string(), numeric_verbs::add_days);
        builtins.insert("addMonths".to_string(), numeric_verbs::add_months);
        builtins.insert("addYears".to_string(), numeric_verbs::add_years);
        builtins.insert("dateDiff".to_string(), numeric_verbs::date_diff);
        builtins.insert("addHours".to_string(), numeric_verbs::add_hours);
        builtins.insert("addMinutes".to_string(), numeric_verbs::add_minutes);
        builtins.insert("addSeconds".to_string(), numeric_verbs::add_seconds);
        builtins.insert("startOfDay".to_string(), numeric_verbs::start_of_day);
        builtins.insert("endOfDay".to_string(), numeric_verbs::end_of_day);
        builtins.insert("startOfMonth".to_string(), numeric_verbs::start_of_month);
        builtins.insert("endOfMonth".to_string(), numeric_verbs::end_of_month);
        builtins.insert("startOfYear".to_string(), numeric_verbs::start_of_year);
        builtins.insert("endOfYear".to_string(), numeric_verbs::end_of_year);
        builtins.insert("dayOfWeek".to_string(), numeric_verbs::day_of_week);
        builtins.insert("weekOfYear".to_string(), numeric_verbs::week_of_year);
        builtins.insert("quarter".to_string(), numeric_verbs::quarter);
        builtins.insert("isLeapYear".to_string(), numeric_verbs::is_leap_year_verb);
        builtins.insert("isBefore".to_string(), numeric_verbs::is_before);
        builtins.insert("isAfter".to_string(), numeric_verbs::is_after);
        builtins.insert("isBetween".to_string(), numeric_verbs::is_between);
        builtins.insert("toUnix".to_string(), numeric_verbs::to_unix);
        builtins.insert("fromUnix".to_string(), numeric_verbs::from_unix);
        builtins.insert("daysBetweenDates".to_string(), numeric_verbs::days_between_dates);
        builtins.insert("ageFromDate".to_string(), numeric_verbs::age_from_date);

        // Financial verbs (20)
        builtins.insert("log".to_string(), numeric_verbs::log_verb);
        builtins.insert("ln".to_string(), numeric_verbs::ln);
        builtins.insert("log10".to_string(), numeric_verbs::log10);
        builtins.insert("exp".to_string(), numeric_verbs::exp_verb);
        builtins.insert("pow".to_string(), numeric_verbs::pow_verb);
        builtins.insert("sqrt".to_string(), numeric_verbs::sqrt);
        builtins.insert("compound".to_string(), numeric_verbs::compound);
        builtins.insert("discount".to_string(), numeric_verbs::discount);
        builtins.insert("pmt".to_string(), numeric_verbs::pmt);
        builtins.insert("fv".to_string(), numeric_verbs::fv);
        builtins.insert("pv".to_string(), numeric_verbs::pv);
        builtins.insert("std".to_string(), numeric_verbs::std_verb);
        builtins.insert("variance".to_string(), numeric_verbs::variance_verb);
        builtins.insert("median".to_string(), numeric_verbs::median);
        builtins.insert("mode".to_string(), numeric_verbs::mode_verb);
        builtins.insert("clamp".to_string(), numeric_verbs::clamp);
        builtins.insert("interpolate".to_string(), numeric_verbs::interpolate);
        builtins.insert("weightedAvg".to_string(), numeric_verbs::weighted_avg);
        builtins.insert("npv".to_string(), numeric_verbs::npv);
        builtins.insert("irr".to_string(), numeric_verbs::irr);
        builtins.insert("mod".to_string(), numeric_verbs::mod_verb);
        builtins.insert("stdSample".to_string(), numeric_verbs::std_sample);
        builtins.insert("varianceSample".to_string(), numeric_verbs::variance_sample);
        builtins.insert("percentile".to_string(), numeric_verbs::percentile);
        builtins.insert("quantile".to_string(), numeric_verbs::quantile);
        builtins.insert("covariance".to_string(), numeric_verbs::covariance);
        builtins.insert("correlation".to_string(), numeric_verbs::correlation);
        builtins.insert("rate".to_string(), numeric_verbs::rate_verb);
        builtins.insert("nper".to_string(), numeric_verbs::nper_verb);
        builtins.insert("depreciation".to_string(), numeric_verbs::depreciation_verb);
        builtins.insert("zscore".to_string(), numeric_verbs::zscore);

        // Encoding (additional)
        builtins.insert("sha1".to_string(), verb_sha1);
        builtins.insert("sha512".to_string(), verb_sha512);

        // Generation (additional)
        builtins.insert("nanoid".to_string(), verb_nanoid);

        // DateTime (additional)
        builtins.insert("isValidDate".to_string(), verb_is_valid_date);
        builtins.insert("formatLocaleDate".to_string(), verb_format_locale_date);

        // Timeseries (additional)
        builtins.insert("lag".to_string(), collection_verbs::lag);
        builtins.insert("lead".to_string(), collection_verbs::lead);

        // Array (additional)
        builtins.insert("rank".to_string(), collection_verbs::rank);
        builtins.insert("fillMissing".to_string(), collection_verbs::fill_missing);

        // Encoding (additional)
        builtins.insert("jsonPath".to_string(), verb_json_path);

        // New verbs
        builtins.insert("reduce".to_string(), collection_verbs::reduce);
        builtins.insert("pivot".to_string(), collection_verbs::pivot);
        builtins.insert("unpivot".to_string(), collection_verbs::unpivot);
        builtins.insert("formatPhone".to_string(), string_verbs::verb_format_phone);
        builtins.insert("movingAvg".to_string(), numeric_verbs::moving_avg);
        builtins.insert("businessDays".to_string(), numeric_verbs::business_days);
        builtins.insert("nextBusinessDay".to_string(), numeric_verbs::next_business_day);
        builtins.insert("formatDuration".to_string(), numeric_verbs::format_duration);
        builtins.insert("convertUnit".to_string(), numeric_verbs::convert_unit);
}

// ─────────────────────────────────────────────────────────────────────────────
// Built-in verb implementations (core set)
// ─────────────────────────────────────────────────────────────────────────────

fn verb_concat(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    let mut result = String::new();
    for arg in args {
        match arg {
            DynValue::String(s) => result.push_str(s),
            DynValue::Integer(n) => result.push_str(&n.to_string()),
            DynValue::Float(n) => result.push_str(&n.to_string()),
            DynValue::Bool(b) => result.push_str(&b.to_string()),
            DynValue::Null => {}
            _ => { use std::fmt::Write; let _ = write!(result, "{arg:?}"); }
        }
    }
    Ok(DynValue::String(result))
}

fn verb_upper(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    match args.first() {
        Some(DynValue::String(s)) => Ok(DynValue::String(s.to_uppercase())),
        Some(DynValue::Null) => Ok(DynValue::Null),
        _ => Err("upper: expected string argument".to_string()),
    }
}

fn verb_lower(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    match args.first() {
        Some(DynValue::String(s)) => Ok(DynValue::String(s.to_lowercase())),
        Some(DynValue::Null) => Ok(DynValue::Null),
        _ => Err("lower: expected string argument".to_string()),
    }
}

fn verb_trim(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    match args.first() {
        Some(DynValue::String(s)) => Ok(DynValue::String(s.trim().to_string())),
        Some(DynValue::Null) => Ok(DynValue::Null),
        _ => Err("trim: expected string argument".to_string()),
    }
}

fn verb_coalesce(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    for arg in args {
        let dominated = matches!(arg, DynValue::Null) || matches!(arg, DynValue::String(s) if s.is_empty());
        if !dominated {
            return Ok(arg.clone());
        }
    }
    Ok(DynValue::Null)
}

fn verb_coerce_string(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    match args.first() {
        Some(DynValue::String(s)) => Ok(DynValue::String(s.clone())),
        Some(DynValue::Integer(n)) => Ok(DynValue::String(n.to_string())),
        Some(DynValue::Float(n)) => Ok(DynValue::String(n.to_string())),
        Some(DynValue::Bool(b)) => Ok(DynValue::String(b.to_string())),
        Some(DynValue::Null) => Ok(DynValue::Null),
        _ => Err("coerceString: unsupported type".to_string()),
    }
}

fn verb_coerce_number(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    match args.first() {
        Some(DynValue::Integer(n)) => Ok(DynValue::Float(*n as f64)),
        Some(DynValue::Float(n)) => Ok(DynValue::Float(*n)),
        Some(DynValue::String(s)) => {
            s.parse::<f64>()
                .map(DynValue::Float)
                .map_err(|_| format!("coerceNumber: cannot parse '{s}' as number"))
        }
        Some(DynValue::Bool(b)) => Ok(DynValue::Float(if *b { 1.0 } else { 0.0 })),
        Some(DynValue::Null) => Ok(DynValue::Null),
        _ => Err("coerceNumber: unsupported type".to_string()),
    }
}

fn verb_coerce_boolean(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    match args.first() {
        Some(DynValue::Bool(b)) => Ok(DynValue::Bool(*b)),
        Some(DynValue::String(s)) => {
            let lower = s.trim().to_lowercase();
            Ok(DynValue::Bool(
                !matches!(lower.as_str(), "" | "false" | "0" | "no" | "n" | "off"),
            ))
        }
        Some(DynValue::Integer(n)) => Ok(DynValue::Bool(*n != 0)),
        Some(DynValue::Float(n)) => Ok(DynValue::Bool(*n != 0.0)),
        Some(DynValue::Null) => Ok(DynValue::Bool(false)),
        _ => Err("coerceBoolean: unsupported type".to_string()),
    }
}

fn dyn_values_equal(a: &DynValue, b: &DynValue) -> bool {
    match (a, b) {
        // Cross-type numeric comparison
        (DynValue::Integer(x), DynValue::Float(y)) => (*x as f64) == *y,
        (DynValue::Float(x), DynValue::Integer(y)) => *x == (*y as f64),
        // String-number coercion
        (DynValue::String(s), DynValue::Integer(n)) | (DynValue::Integer(n), DynValue::String(s)) => s.parse::<i64>().ok() == Some(*n),
        (DynValue::String(s), DynValue::Float(n)) | (DynValue::Float(n), DynValue::String(s)) => s.parse::<f64>().ok() == Some(*n),
        // Default structural equality
        _ => a == b,
    }
}

fn verb_eq(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    if args.len() < 2 {
        return Err("eq: requires 2 arguments".to_string());
    }
    Ok(DynValue::Bool(dyn_values_equal(&args[0], &args[1])))
}

fn verb_ne(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    if args.len() < 2 {
        return Err("ne: requires 2 arguments".to_string());
    }
    Ok(DynValue::Bool(!dyn_values_equal(&args[0], &args[1])))
}

fn verb_not(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    match args.first() {
        Some(DynValue::Bool(b)) => Ok(DynValue::Bool(!b)),
        Some(DynValue::Null) => Ok(DynValue::Bool(true)),
        Some(DynValue::Integer(n)) => Ok(DynValue::Bool(*n == 0)),
        Some(DynValue::Float(n)) => Ok(DynValue::Bool(*n == 0.0)),
        Some(DynValue::String(s)) => Ok(DynValue::Bool(s.is_empty() || s == "false")),
        Some(DynValue::Array(a)) => Ok(DynValue::Bool(a.is_empty())),
        _ => Ok(DynValue::Bool(false)),
    }
}

fn verb_is_null(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    match args.first() {
        Some(v) => Ok(DynValue::Bool(v.is_null())),
        None => Ok(DynValue::Bool(true)),
    }
}

fn verb_capitalize(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    match args.first() {
        Some(DynValue::String(s)) => {
            let mut chars = s.chars();
            let result = match chars.next() {
                Some(c) => c.to_uppercase().to_string() + chars.as_str(),
                None => String::new(),
            };
            Ok(DynValue::String(result))
        }
        Some(DynValue::Null) => Ok(DynValue::Null),
        _ => Err("capitalize: expected string argument".to_string()),
    }
}

fn verb_replace(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    if args.len() < 3 {
        return Err("replace: requires 3 arguments (string, search, replacement)".to_string());
    }
    match (&args[0], &args[1], &args[2]) {
        (DynValue::String(s), DynValue::String(search), DynValue::String(replacement)) => {
            Ok(DynValue::String(s.replace(search.as_str(), replacement)))
        }
        _ => Err("replace: expected string arguments".to_string()),
    }
}

fn verb_substring(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    if args.len() < 2 {
        return Err("substring: requires at least 2 arguments (string, start[, end])".to_string());
    }
    let DynValue::String(s) = &args[0] else { return Err("substring: first argument must be a string".to_string()) };
    let start_val = args[1].as_i64().or_else(|| args[1].as_f64().map(|f| f as i64))
        .ok_or("substring: start must be numeric")?;
    let start = (start_val.max(0) as usize).min(s.len());
    let end = if args.len() >= 3 {
        let end_val = args[2].as_i64().or_else(|| args[2].as_f64().map(|f| f as i64))
            .ok_or("substring: end must be numeric")?;
        (end_val.max(0) as usize).min(s.len())
    } else {
        s.len()
    };
    if start >= end {
        return Ok(DynValue::String(String::new()));
    }
    Ok(DynValue::String(s[start..end].to_string()))
}

fn verb_length(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    match args.first() {
        Some(DynValue::String(s)) => Ok(DynValue::Integer(s.len() as i64)),
        Some(DynValue::Array(a)) => Ok(DynValue::Integer(a.len() as i64)),
        Some(DynValue::Null) => Ok(DynValue::Integer(0)),
        _ => Err("length: expected string or array".to_string()),
    }
}

fn coerce_num(v: &DynValue) -> Option<DynValue> {
    match v {
        DynValue::Integer(_) | DynValue::Float(_) => Some(v.clone()),
        DynValue::Currency(f, _, _) | DynValue::Percent(f) => Some(DynValue::Float(*f)),
        DynValue::CurrencyRaw(s, _, _) | DynValue::FloatRaw(s) => s.parse::<f64>().ok().map(DynValue::Float),
        DynValue::String(s) => {
            if let Ok(i) = s.parse::<i64>() {
                Some(DynValue::Integer(i))
            } else if let Ok(f) = s.parse::<f64>() {
                Some(DynValue::Float(f))
            } else {
                None
            }
        }
        DynValue::Bool(b) => Some(DynValue::Integer(i64::from(*b))),
        _ => None,
    }
}

/// Coerce any `DynValue` to a string representation (matches TS toString).
/// Public accessor for `coerce_str` (used by engine for directive processing).
pub fn coerce_str_pub(v: &DynValue) -> String {
    coerce_str(v)
}

fn coerce_str(v: &DynValue) -> String {
    match v {
        DynValue::String(s)
        | DynValue::CurrencyRaw(s, _, _)
        | DynValue::FloatRaw(s)
        | DynValue::Date(s) | DynValue::Timestamp(s) | DynValue::Time(s) | DynValue::Duration(s)
        | DynValue::Reference(s)
        | DynValue::Binary(s) => s.clone(),
        DynValue::Integer(n) => n.to_string(),
        DynValue::Float(f) | DynValue::Currency(f, _, _) | DynValue::Percent(f) => f.to_string(),
        DynValue::Bool(b) => b.to_string(),
        DynValue::Null => String::new(),
        DynValue::Array(a) => format!("[{} items]", a.len()),
        DynValue::Object(_) => "[object]".to_string(),
    }
}

fn verb_add(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    if args.len() < 2 {
        return Err("add: requires 2 arguments".to_string());
    }
    let a = coerce_num(&args[0]).ok_or("add: expected numeric arguments")?;
    let b = coerce_num(&args[1]).ok_or("add: expected numeric arguments")?;
    if let (DynValue::Integer(x), DynValue::Integer(y)) = (&a, &b) { Ok(DynValue::Integer(x + y)) } else {
        let x = a.as_f64().unwrap_or(0.0);
        let y = b.as_f64().unwrap_or(0.0);
        Ok(DynValue::Float(x + y))
    }
}

fn verb_subtract(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    if args.len() < 2 {
        return Err("subtract: requires 2 arguments".to_string());
    }
    let a = coerce_num(&args[0]).ok_or("subtract: expected numeric arguments")?;
    let b = coerce_num(&args[1]).ok_or("subtract: expected numeric arguments")?;
    if let (DynValue::Integer(x), DynValue::Integer(y)) = (&a, &b) { Ok(DynValue::Integer(x - y)) } else {
        let x = a.as_f64().unwrap_or(0.0);
        let y = b.as_f64().unwrap_or(0.0);
        Ok(DynValue::Float(x - y))
    }
}

fn verb_multiply(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    if args.len() < 2 {
        return Err("multiply: requires 2 arguments".to_string());
    }
    let a = coerce_num(&args[0]).ok_or("multiply: expected numeric arguments")?;
    let b = coerce_num(&args[1]).ok_or("multiply: expected numeric arguments")?;
    if let (DynValue::Integer(x), DynValue::Integer(y)) = (&a, &b) { Ok(DynValue::Integer(x * y)) } else {
        let x = a.as_f64().unwrap_or(0.0);
        let y = b.as_f64().unwrap_or(0.0);
        Ok(DynValue::Float(x * y))
    }
}

fn verb_divide(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    if args.len() < 2 {
        return Err("divide: requires 2 arguments".to_string());
    }
    let x = coerce_num(&args[0]).map_or(0.0, |v| v.as_f64().unwrap_or(0.0));
    let y = coerce_num(&args[1]).map_or(0.0, |v| v.as_f64().unwrap_or(0.0));
    if y == 0.0 {
        return Ok(DynValue::Null);
    }
    let result = x / y;
    // TS divide always returns number type (not integer)
    Ok(DynValue::Float(result))
}

fn verb_abs(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    match args.first() {
        Some(DynValue::Integer(n)) => Ok(DynValue::Integer(n.abs())),
        Some(DynValue::Float(n)) => Ok(DynValue::Float(n.abs())),
        Some(DynValue::String(s)) => {
            if let Ok(i) = s.parse::<i64>() {
                Ok(DynValue::Integer(i.abs()))
            } else if let Ok(f) = s.parse::<f64>() {
                Ok(DynValue::Float(f.abs()))
            } else {
                Err("abs: cannot parse string as number".to_string())
            }
        }
        _ => Err("abs: expected numeric argument".to_string()),
    }
}

fn verb_round(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    let val = match args.first() {
        Some(DynValue::Float(n)) => *n,
        Some(DynValue::Integer(n)) => return Ok(DynValue::Integer(*n)),
        Some(DynValue::String(s)) => s.parse::<f64>().map_err(|_| "round: cannot parse string as number".to_string())?,
        _ => return Err("round: expected numeric argument".to_string()),
    };
    let places = args.get(1).and_then(super::super::types::transform::DynValue::as_i64).unwrap_or(0);
    let factor = 10_f64.powi(places as i32);
    let result = (val * factor).round() / factor;
    // Promote to Integer when result has no fractional part (matches TypeScript behavior)
    if result.fract() == 0.0 && result.abs() < i64::MAX as f64 {
        Ok(DynValue::Integer(result as i64))
    } else {
        Ok(DynValue::Float(result))
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Core verbs (additional)
// ─────────────────────────────────────────────────────────────────────────────

fn verb_trim_left(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    match args.first() {
        Some(DynValue::String(s)) => Ok(DynValue::String(s.trim_start().to_string())),
        Some(DynValue::Null) => Ok(DynValue::Null),
        _ => Err("trimLeft: expected string argument".to_string()),
    }
}

fn verb_trim_right(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    match args.first() {
        Some(DynValue::String(s)) => Ok(DynValue::String(s.trim_end().to_string())),
        Some(DynValue::Null) => Ok(DynValue::Null),
        _ => Err("trimRight: expected string argument".to_string()),
    }
}

fn verb_if_null(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    if args.len() < 2 {
        return Err("ifNull: requires 2 arguments".to_string());
    }
    if args[0].is_null() {
        Ok(args[1].clone())
    } else {
        Ok(args[0].clone())
    }
}

fn verb_if_empty(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    if args.len() < 2 {
        return Err("ifEmpty: requires 2 arguments".to_string());
    }
    let is_empty = match &args[0] {
        DynValue::Null => true,
        DynValue::String(s) => s.is_empty(),
        _ => false,
    };
    if is_empty {
        Ok(args[1].clone())
    } else {
        Ok(args[0].clone())
    }
}

/// Helper: determine if a `DynValue` is "truthy".
/// Truthy = non-null, non-false, non-empty-string, non-zero.
fn is_truthy(val: &DynValue) -> bool {
    match val {
        DynValue::Null => false,
        DynValue::Bool(b) => *b,
        DynValue::String(s) | DynValue::Reference(s) | DynValue::Binary(s)
        | DynValue::Date(s) | DynValue::Timestamp(s) | DynValue::Time(s)
        | DynValue::Duration(s) => !s.is_empty(),
        DynValue::Integer(n) => *n != 0,
        DynValue::Float(n) | DynValue::Currency(n, _, _) | DynValue::Percent(n) => *n != 0.0,
        DynValue::FloatRaw(s) | DynValue::CurrencyRaw(s, _, _) => !s.is_empty() && s != "0",
        DynValue::Array(_) | DynValue::Object(_) => true,
    }
}

fn verb_if_else(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    if args.len() < 3 {
        return Err("ifElse: requires 3 arguments".to_string());
    }
    if is_truthy(&args[0]) {
        Ok(args[1].clone())
    } else {
        Ok(args[2].clone())
    }
}

/// Helper: compare a `DynValue` to a key argument for table lookups.
/// Performs string-based comparison to handle cross-type matching (e.g., Integer vs String).
fn dyn_matches_key(cell: &DynValue, key: &DynValue) -> bool {
    // Direct equality first
    if cell == key {
        return true;
    }
    // Cross-type string comparison
    let cell_str = match cell {
        DynValue::String(s) => s.clone(),
        DynValue::Integer(n) => n.to_string(),
        DynValue::Float(n) => n.to_string(),
        DynValue::Bool(b) => b.to_string(),
        DynValue::Null => return matches!(key, DynValue::Null),
        _ => return false,
    };
    let key_str = match key {
        DynValue::String(s) => s.clone(),
        DynValue::Integer(n) => n.to_string(),
        DynValue::Float(n) => n.to_string(),
        DynValue::Bool(b) => b.to_string(),
        _ => return false,
    };
    cell_str == key_str
}

/// Helper: perform a table lookup by matching key columns and returning a result column.
/// `table_ref` is "`TABLE_NAME.result_column`", keys are the values to match.
fn do_table_lookup<'a>(
    table_ref: &str,
    keys: &[DynValue],
    tables: &'a HashMap<String, crate::types::transform::LookupTable>,
) -> Option<&'a DynValue> {
    // Parse "TABLE_NAME.result_column"
    let (table_name, result_col) = if let Some(dot) = table_ref.find('.') {
        (&table_ref[..dot], &table_ref[dot + 1..])
    } else {
        return None;
    };

    let table = tables.get(table_name)?;

    // Find result column index
    let result_idx = table.columns.iter().position(|c| c == result_col)?;

    // Key columns are all columns except the result column.
    // With a single lookup key, try matching against ALL non-result columns
    // (supports reverse lookup where the match column isn't the first column).
    let num_keys = keys.len();

    // Find matching row
    for row in &table.rows {
        if row.len() <= result_idx {
            continue;
        }
        if num_keys == 1 {
            // Single key: match against ANY non-result column (reverse lookup support)
            let key = &keys[0];
            let mut matched = false;
            for (col_idx, cell) in row.iter().enumerate() {
                if col_idx == result_idx {
                    continue; // skip the result column
                }
                if dyn_matches_key(cell, key) {
                    matched = true;
                    break;
                }
            }
            if matched {
                return Some(&row[result_idx]);
            }
        } else {
            // Multiple keys: match against first N columns in order
            let mut all_match = true;
            for (i, key) in keys.iter().enumerate() {
                if i >= row.len() {
                    all_match = false;
                    break;
                }
                if !dyn_matches_key(&row[i], key) {
                    all_match = false;
                    break;
                }
            }
            if all_match && num_keys > 0 {
                return Some(&row[result_idx]);
            }
        }
    }

    None
}

fn verb_lookup(args: &[DynValue], ctx: &VerbContext) -> Result<DynValue, String> {
    if args.len() < 2 {
        return Err("lookup: requires at least 2 arguments (table.column, key...)".to_string());
    }
    let table_ref = match &args[0] {
        DynValue::String(s) => s.clone(),
        _ => return Err("lookup: first argument must be a string table reference".to_string()),
    };
    let keys = &args[1..];

    if let Some(val) = do_table_lookup(&table_ref, keys, &ctx.tables) { Ok(val.clone()) } else {
        // Check for table-level default
        let table_name = table_ref.split('.').next().unwrap_or(&table_ref);
        if let Some(table) = ctx.tables.get(table_name) {
            if let Some(ref default) = table.default {
                return Ok(default.clone());
            }
        }
        Ok(DynValue::Null)
    }
}

fn verb_lookup_default(args: &[DynValue], ctx: &VerbContext) -> Result<DynValue, String> {
    if args.len() < 3 {
        return Err("lookupDefault: requires at least 3 arguments (table.column, key..., default)".to_string());
    }
    let table_ref = match &args[0] {
        DynValue::String(s) => s.clone(),
        _ => return Err("lookupDefault: first argument must be a string table reference".to_string()),
    };
    // Last arg is the default value; middle args are lookup keys
    let default_val = &args[args.len() - 1];
    let keys = &args[1..args.len() - 1];
    if keys.is_empty() {
        return Err("lookupDefault: requires at least one lookup key".to_string());
    }

    match do_table_lookup(&table_ref, keys, &ctx.tables) {
        Some(val) => Ok(val.clone()),
        None => Ok(default_val.clone()),
    }
}


// ─────────────────────────────────────────────────────────────────────────────
// Logic verbs (additional)
// ─────────────────────────────────────────────────────────────────────────────

fn verb_and(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    if args.len() < 2 {
        return Err("and: requires 2 arguments".to_string());
    }
    match (&args[0], &args[1]) {
        (DynValue::Bool(a), DynValue::Bool(b)) => Ok(DynValue::Bool(*a && *b)),
        _ => Err("and: expected boolean arguments".to_string()),
    }
}

fn verb_or(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    if args.len() < 2 {
        return Err("or: requires 2 arguments".to_string());
    }
    match (&args[0], &args[1]) {
        (DynValue::Bool(a), DynValue::Bool(b)) => Ok(DynValue::Bool(*a || *b)),
        _ => Err("or: expected boolean arguments".to_string()),
    }
}

fn verb_xor(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    if args.len() < 2 {
        return Err("xor: requires 2 arguments".to_string());
    }
    match (&args[0], &args[1]) {
        (DynValue::Bool(a), DynValue::Bool(b)) => Ok(DynValue::Bool(*a ^ *b)),
        _ => Err("xor: expected boolean arguments".to_string()),
    }
}

/// Helper: extract an f64 from a `DynValue` for numeric comparison.
fn to_f64_for_cmp(val: &DynValue) -> Option<f64> {
    match val {
        DynValue::Integer(n) => Some(*n as f64),
        DynValue::Float(n) => Some(*n),
        _ => None,
    }
}

fn verb_lt(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    if args.len() < 2 {
        return Err("lt: requires 2 arguments".to_string());
    }
    // Try numeric comparison first
    if let (Some(a), Some(b)) = (to_f64_for_cmp(&args[0]), to_f64_for_cmp(&args[1])) {
        return Ok(DynValue::Bool(a < b));
    }
    // Fall back to string comparison
    if let (DynValue::String(a), DynValue::String(b)) = (&args[0], &args[1]) {
        return Ok(DynValue::Bool(a < b));
    }
    Err("lt: expected numeric or string arguments".to_string())
}

fn verb_lte(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    if args.len() < 2 {
        return Err("lte: requires 2 arguments".to_string());
    }
    if let (Some(a), Some(b)) = (to_f64_for_cmp(&args[0]), to_f64_for_cmp(&args[1])) {
        return Ok(DynValue::Bool(a <= b));
    }
    if let (DynValue::String(a), DynValue::String(b)) = (&args[0], &args[1]) {
        return Ok(DynValue::Bool(a <= b));
    }
    Err("lte: expected numeric or string arguments".to_string())
}

fn verb_gt(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    if args.len() < 2 {
        return Err("gt: requires 2 arguments".to_string());
    }
    if let (Some(a), Some(b)) = (to_f64_for_cmp(&args[0]), to_f64_for_cmp(&args[1])) {
        return Ok(DynValue::Bool(a > b));
    }
    if let (DynValue::String(a), DynValue::String(b)) = (&args[0], &args[1]) {
        return Ok(DynValue::Bool(a > b));
    }
    Err("gt: expected numeric or string arguments".to_string())
}

fn verb_gte(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    if args.len() < 2 {
        return Err("gte: requires 2 arguments".to_string());
    }
    if let (Some(a), Some(b)) = (to_f64_for_cmp(&args[0]), to_f64_for_cmp(&args[1])) {
        return Ok(DynValue::Bool(a >= b));
    }
    if let (DynValue::String(a), DynValue::String(b)) = (&args[0], &args[1]) {
        return Ok(DynValue::Bool(a >= b));
    }
    Err("gte: expected numeric or string arguments".to_string())
}

fn verb_between(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    if args.len() < 3 {
        return Err("between: requires 3 arguments (value, min, max)".to_string());
    }
    // Try numeric comparison
    if let (Some(val), Some(min), Some(max)) = (
        to_f64_for_cmp(&args[0]),
        to_f64_for_cmp(&args[1]),
        to_f64_for_cmp(&args[2]),
    ) {
        return Ok(DynValue::Bool(val >= min && val <= max));
    }
    // Fall back to string comparison
    if let (DynValue::String(val), DynValue::String(min), DynValue::String(max)) =
        (&args[0], &args[1], &args[2])
    {
        return Ok(DynValue::Bool(val >= min && val <= max));
    }
    Err("between: expected numeric or string arguments".to_string())
}

fn verb_is_string(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    match args.first() {
        Some(v) => Ok(DynValue::Bool(matches!(v, DynValue::String(_)))),
        None => Ok(DynValue::Bool(false)),
    }
}

fn verb_is_number(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    match args.first() {
        Some(v) => Ok(DynValue::Bool(matches!(v, DynValue::Integer(_) | DynValue::Float(_)))),
        None => Ok(DynValue::Bool(false)),
    }
}

fn verb_is_boolean(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    match args.first() {
        Some(v) => Ok(DynValue::Bool(matches!(v, DynValue::Bool(_)))),
        None => Ok(DynValue::Bool(false)),
    }
}

fn verb_is_array(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    match args.first() {
        Some(DynValue::Array(_)) => Ok(DynValue::Bool(true)),
        Some(DynValue::String(s)) => {
            let trimmed = s.trim();
            Ok(DynValue::Bool(trimmed.starts_with('[') && trimmed.ends_with(']')))
        }
        Some(_) | None => Ok(DynValue::Bool(false)),
    }
}

fn verb_is_object(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    match args.first() {
        Some(DynValue::Object(_)) => Ok(DynValue::Bool(true)),
        Some(DynValue::String(s)) => {
            let trimmed = s.trim();
            Ok(DynValue::Bool(trimmed.starts_with('{') && trimmed.ends_with('}')))
        }
        Some(_) | None => Ok(DynValue::Bool(false)),
    }
}

fn verb_is_date(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    match args.first() {
        Some(DynValue::String(s)) => {
            // Check for YYYY-MM-DD pattern
            let is_date = s.len() >= 10 && is_valid_date_prefix(s);
            Ok(DynValue::Bool(is_date))
        }
        _ => Ok(DynValue::Bool(false)),
    }
}

/// Check if a string starts with a valid YYYY-MM-DD date pattern.
fn is_valid_date_prefix(s: &str) -> bool {
    let bytes = s.as_bytes();
    if bytes.len() < 10 {
        return false;
    }
    // YYYY-MM-DD
    bytes[0..4].iter().all(u8::is_ascii_digit)
        && bytes[4] == b'-'
        && bytes[5..7].iter().all(u8::is_ascii_digit)
        && bytes[7] == b'-'
        && bytes[8..10].iter().all(u8::is_ascii_digit)
}

fn verb_type_of(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    let type_name = match args.first() {
        Some(DynValue::Null) | None => "null",
        Some(DynValue::Bool(_)) => "boolean",
        Some(DynValue::String(_)) => "string",
        Some(DynValue::Integer(_)) => "integer",
        Some(DynValue::Float(_) | DynValue::FloatRaw(_)) => "number",
        Some(DynValue::Currency(_, _, _) | DynValue::CurrencyRaw(_, _, _)) => "currency",
        Some(DynValue::Percent(_)) => "percent",
        Some(DynValue::Reference(_)) => "reference",
        Some(DynValue::Binary(_)) => "binary",
        Some(DynValue::Date(_)) => "date",
        Some(DynValue::Timestamp(_)) => "timestamp",
        Some(DynValue::Time(_)) => "time",
        Some(DynValue::Duration(_)) => "duration",
        Some(DynValue::Array(_)) => "array",
        Some(DynValue::Object(_)) => "object",
    };
    Ok(DynValue::String(type_name.to_string()))
}

fn verb_cond(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    // Process condition/value pairs; if odd number of args, last is default
    let mut i = 0;
    while i + 1 < args.len() {
        if is_truthy(&args[i]) {
            return Ok(args[i + 1].clone());
        }
        i += 2;
    }
    // If there's a remaining arg, it's the default
    if i < args.len() {
        return Ok(args[i].clone());
    }
    Ok(DynValue::Null)
}

fn verb_assert(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    if args.is_empty() {
        return Err("assert: requires at least 1 argument".to_string());
    }
    if is_truthy(&args[0]) {
        Ok(args[0].clone())
    } else {
        let message = args.get(1)
            .and_then(|v| v.as_str())
            .unwrap_or("assertion failed");
        Err(format!("assert: {message}"))
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Coercion verbs (additional)
// ─────────────────────────────────────────────────────────────────────────────

fn verb_coerce_integer(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    match args.first() {
        Some(DynValue::Integer(n)) => Ok(DynValue::Integer(*n)),
        Some(DynValue::Float(n)) => Ok(DynValue::Integer(*n as i64)),
        Some(DynValue::String(s)) => {
            // Try parsing as integer directly first
            if let Ok(n) = s.parse::<i64>() {
                return Ok(DynValue::Integer(n));
            }
            // Try parsing as float then truncating
            if let Ok(n) = s.parse::<f64>() {
                return Ok(DynValue::Integer(n as i64));
            }
            Err(format!("coerceInteger: cannot parse '{s}' as integer"))
        }
        Some(DynValue::Bool(b)) => Ok(DynValue::Integer(i64::from(*b))),
        Some(DynValue::Null) => Ok(DynValue::Null),
        _ => Err("coerceInteger: unsupported type".to_string()),
    }
}

fn verb_coerce_date(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    match args.first() {
        Some(DynValue::String(s)) => {
            // Accept YYYY-MM-DD (10 chars) or strip time portion from timestamps
            let date_part = if s.len() >= 10 && is_valid_date_prefix(s) {
                &s[..10]
            } else {
                return Err(format!("coerceDate: '{s}' is not a valid date"));
            };
            let month: u32 = date_part[5..7].parse().unwrap_or(0);
            let day: u32 = date_part[8..10].parse().unwrap_or(0);
            if (1..=12).contains(&month) && (1..=31).contains(&day) {
                Ok(DynValue::Date(date_part.to_string()))
            } else {
                Err(format!("coerceDate: '{s}' is not a valid date"))
            }
        }
        Some(DynValue::Integer(n)) => {
            // Unix timestamp (seconds) — convert to date
            let secs = *n;
            let days = secs / 86400 + 719_468;
            let era = if days >= 0 { days } else { days - 146_096 } / 146_097;
            let doe = (days - era * 146_097) as u32;
            let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
            let y = i64::from(yoe) + era * 400;
            let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
            let mp = (5 * doy + 2) / 153;
            let d = doy - (153 * mp + 2) / 5 + 1;
            let m = if mp < 10 { mp + 3 } else { mp - 9 };
            let y = if m <= 2 { y + 1 } else { y };
            Ok(DynValue::String(format!("{y:04}-{m:02}-{d:02}")))
        }
        Some(DynValue::Null) => Ok(DynValue::Null),
        _ => Err("coerceDate: expected string argument".to_string()),
    }
}

fn verb_coerce_timestamp(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    match args.first() {
        Some(DynValue::String(s)) => {
            // Accept YYYY-MM-DDThh:mm:ss or YYYY-MM-DDThh:mm:ssZ or with timezone offset
            if s.len() >= 19 && is_valid_date_prefix(s) {
                let bytes = s.as_bytes();
                if (bytes[10] == b'T' || bytes[10] == b' ')
                    && bytes[11..13].iter().all(u8::is_ascii_digit)
                    && bytes[13] == b':'
                    && bytes[14..16].iter().all(u8::is_ascii_digit)
                    && bytes[16] == b':'
                    && bytes[17..19].iter().all(u8::is_ascii_digit)
                {
                    return Ok(DynValue::String(s.clone()));
                }
            }
            // Accept YYYY-MM-DD and append T00:00:00
            if s.len() == 10 && is_valid_date_prefix(s) {
                return Ok(DynValue::String(format!("{s}T00:00:00")));
            }
            Err(format!("coerceTimestamp: '{s}' is not a valid timestamp"))
        }
        Some(DynValue::Null) => Ok(DynValue::Null),
        _ => Err("coerceTimestamp: expected string argument".to_string()),
    }
}

fn verb_try_coerce(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    match args.first() {
        Some(DynValue::String(s)) => {
            // Try integer
            if let Ok(n) = s.parse::<i64>() {
                return Ok(DynValue::Integer(n));
            }
            // Try float
            if let Ok(n) = s.parse::<f64>() {
                return Ok(DynValue::Float(n));
            }
            // Try boolean
            match s.as_str() {
                "true" => return Ok(DynValue::Bool(true)),
                "false" => return Ok(DynValue::Bool(false)),
                _ => {}
            }
            // Try date (YYYY-MM-DD)
            if s.len() == 10 && is_valid_date_prefix(s) {
                return Ok(DynValue::Date(s.clone()));
            }
            // Keep as string
            Ok(DynValue::String(s.clone()))
        }
        Some(other) => Ok(other.clone()),
        None => Ok(DynValue::Null),
    }
}

fn verb_to_array(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    match args.first() {
        Some(DynValue::Array(arr)) => Ok(DynValue::Array(arr.clone())),
        Some(val) => Ok(DynValue::Array(vec![val.clone()])),
        None => Ok(DynValue::Array(vec![])),
    }
}

fn verb_to_object(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    match args.first() {
        Some(DynValue::Object(obj)) => Ok(DynValue::Object(obj.clone())),
        Some(DynValue::Array(arr)) => {
            // Convert array of [key, value] pairs to object
            let mut entries = Vec::new();
            for item in arr {
                match item {
                    DynValue::Array(pair) if pair.len() >= 2 => {
                        let key = match &pair[0] {
                            DynValue::String(s) => s.clone(),
                            DynValue::Integer(n) => n.to_string(),
                            other => format!("{other:?}"),
                        };
                        entries.push((key, pair[1].clone()));
                    }
                    _ => return Err("toObject: array elements must be [key, value] pairs".to_string()),
                }
            }
            Ok(DynValue::Object(entries))
        }
        Some(DynValue::Null) => Ok(DynValue::Null),
        _ => Err("toObject: expected array of [key, value] pairs".to_string()),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// SHA-1, SHA-512 hashing (simple implementations)
// ─────────────────────────────────────────────────────────────────────────────

fn verb_sha1(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    let s = match args.first() {
        Some(DynValue::String(s)) => s.as_str(),
        Some(DynValue::Null) => return Ok(DynValue::Null),
        _ => return Err("sha1: expected string argument".to_string()),
    };
    Ok(DynValue::String(sha1_hex(s.as_bytes())))
}

fn verb_sha512(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    let s = match args.first() {
        Some(DynValue::String(s)) => s.as_str(),
        Some(DynValue::Null) => return Ok(DynValue::Null),
        _ => return Err("sha512: expected string argument".to_string()),
    };
    Ok(DynValue::String(sha512_hex(s.as_bytes())))
}

/// Minimal SHA-1 implementation (FIPS 180-4).
fn sha1_hex(data: &[u8]) -> String {
    let mut h0: u32 = 0x6745_2301;
    let mut h1: u32 = 0xEFCD_AB89;
    let mut h2: u32 = 0x98BA_DCFE;
    let mut h3: u32 = 0x1032_5476;
    let mut h4: u32 = 0xC3D2_E1F0;

    let bit_len = (data.len() as u64) * 8;
    let mut msg = data.to_vec();
    msg.push(0x80);
    while (msg.len() % 64) != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bit_len.to_be_bytes());

    for chunk in msg.chunks(64) {
        let mut w = [0u32; 80];
        for (i, wi) in w.iter_mut().enumerate().take(16) {
            *wi = u32::from_be_bytes([chunk[4 * i], chunk[4 * i + 1], chunk[4 * i + 2], chunk[4 * i + 3]]);
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }
        let (mut a, mut b, mut c, mut d, mut e) = (h0, h1, h2, h3, h4);
        for (i, &wi) in w.iter().enumerate() {
            let (f, k) = match i {
                0..=19 => ((b & c) | ((!b) & d), 0x5A82_7999_u32),
                20..=39 => (b ^ c ^ d, 0x6ED9_EBA1_u32),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1B_BCDC_u32),
                _ => (b ^ c ^ d, 0xCA62_C1D6_u32),
            };
            let temp = a.rotate_left(5).wrapping_add(f).wrapping_add(e).wrapping_add(k).wrapping_add(wi);
            e = d; d = c; c = b.rotate_left(30); b = a; a = temp;
        }
        h0 = h0.wrapping_add(a); h1 = h1.wrapping_add(b);
        h2 = h2.wrapping_add(c); h3 = h3.wrapping_add(d); h4 = h4.wrapping_add(e);
    }
    format!("{h0:08x}{h1:08x}{h2:08x}{h3:08x}{h4:08x}")
}

/// Minimal SHA-512 implementation (FIPS 180-4).
fn sha512_hex(data: &[u8]) -> String {
    const K: [u64; 80] = [
        0x428a_2f98_d728_ae22, 0x7137_4491_23ef_65cd, 0xb5c0_fbcf_ec4d_3b2f, 0xe9b5_dba5_8189_dbbc,
        0x3956_c25b_f348_b538, 0x59f1_11f1_b605_d019, 0x923f_82a4_af19_4f9b, 0xab1c_5ed5_da6d_8118,
        0xd807_aa98_a303_0242, 0x1283_5b01_4570_6fbe, 0x2431_85be_4ee4_b28c, 0x550c_7dc3_d5ff_b4e2,
        0x72be_5d74_f27b_896f, 0x80de_b1fe_3b16_96b1, 0x9bdc_06a7_25c7_1235, 0xc19b_f174_cf69_2694,
        0xe49b_69c1_9ef1_4ad2, 0xefbe_4786_384f_25e3, 0x0fc1_9dc6_8b8c_d5b5, 0x240c_a1cc_77ac_9c65,
        0x2de9_2c6f_592b_0275, 0x4a74_84aa_6ea6_e483, 0x5cb0_a9dc_bd41_fbd4, 0x76f9_88da_8311_53b5,
        0x983e_5152_ee66_dfab, 0xa831_c66d_2db4_3210, 0xb003_27c8_98fb_213f, 0xbf59_7fc7_beef_0ee4,
        0xc6e0_0bf3_3da8_8fc2, 0xd5a7_9147_930a_a725, 0x06ca_6351_e003_826f, 0x1429_2967_0a0e_6e70,
        0x27b7_0a85_46d2_2ffc, 0x2e1b_2138_5c26_c926, 0x4d2c_6dfc_5ac4_2aed, 0x5338_0d13_9d95_b3df,
        0x650a_7354_8baf_63de, 0x766a_0abb_3c77_b2a8, 0x81c2_c92e_47ed_aee6, 0x9272_2c85_1482_353b,
        0xa2bf_e8a1_4cf1_0364, 0xa81a_664b_bc42_3001, 0xc24b_8b70_d0f8_9791, 0xc76c_51a3_0654_be30,
        0xd192_e819_d6ef_5218, 0xd699_0624_5565_a910, 0xf40e_3585_5771_202a, 0x106a_a070_32bb_d1b8,
        0x19a4_c116_b8d2_d0c8, 0x1e37_6c08_5141_ab53, 0x2748_774c_df8e_eb99, 0x34b0_bcb5_e19b_48a8,
        0x391c_0cb3_c5c9_5a63, 0x4ed8_aa4a_e341_8acb, 0x5b9c_ca4f_7763_e373, 0x682e_6ff3_d6b2_b8a3,
        0x748f_82ee_5def_b2fc, 0x78a5_636f_4317_2f60, 0x84c8_7814_a1f0_ab72, 0x8cc7_0208_1a64_39ec,
        0x90be_fffa_2363_1e28, 0xa450_6ceb_de82_bde9, 0xbef9_a3f7_b2c6_7915, 0xc671_78f2_e372_532b,
        0xca27_3ece_ea26_619c, 0xd186_b8c7_21c0_c207, 0xeada_7dd6_cde0_eb1e, 0xf57d_4f7f_ee6e_d178,
        0x06f0_67aa_7217_6fba, 0x0a63_7dc5_a2c8_98a6, 0x113f_9804_bef9_0dae, 0x1b71_0b35_131c_471b,
        0x28db_77f5_2304_7d84, 0x32ca_ab7b_40c7_2493, 0x3c9e_be0a_15c9_bebc, 0x431d_67c4_9c10_0d4c,
        0x4cc5_d4be_cb3e_42b6, 0x597f_299c_fc65_7e2a, 0x5fcb_6fab_3ad6_faec, 0x6c44_198c_4a47_5817,
    ];

    let mut h: [u64; 8] = [
        0x6a09_e667_f3bc_c908, 0xbb67_ae85_84ca_a73b, 0x3c6e_f372_fe94_f82b, 0xa54f_f53a_5f1d_36f1,
        0x510e_527f_ade6_82d1, 0x9b05_688c_2b3e_6c1f, 0x1f83_d9ab_fb41_bd6b, 0x5be0_cd19_137e_2179,
    ];

    let bit_len = (data.len() as u128) * 8;
    let mut msg = data.to_vec();
    msg.push(0x80);
    while (msg.len() % 128) != 112 {
        msg.push(0);
    }
    msg.extend_from_slice(&bit_len.to_be_bytes());

    for chunk in msg.chunks(128) {
        let mut w = [0u64; 80];
        for (i, wi) in w.iter_mut().enumerate().take(16) {
            let off = i * 8;
            *wi = u64::from_be_bytes([
                chunk[off], chunk[off+1], chunk[off+2], chunk[off+3],
                chunk[off+4], chunk[off+5], chunk[off+6], chunk[off+7],
            ]);
        }
        for i in 16..80 {
            let s0 = w[i-15].rotate_right(1) ^ w[i-15].rotate_right(8) ^ (w[i-15] >> 7);
            let s1 = w[i-2].rotate_right(19) ^ w[i-2].rotate_right(61) ^ (w[i-2] >> 6);
            w[i] = w[i-16].wrapping_add(s0).wrapping_add(w[i-7]).wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = h;
        for i in 0..80 {
            let s1 = e.rotate_right(14) ^ e.rotate_right(18) ^ e.rotate_right(41);
            let ch = (e & f) ^ ((!e) & g);
            let temp1 = hh.wrapping_add(s1).wrapping_add(ch).wrapping_add(K[i]).wrapping_add(w[i]);
            let s0 = a.rotate_right(28) ^ a.rotate_right(34) ^ a.rotate_right(39);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = s0.wrapping_add(maj);
            hh = g; g = f; f = e; e = d.wrapping_add(temp1);
            d = c; c = b; b = a; a = temp1.wrapping_add(temp2);
        }
        h[0] = h[0].wrapping_add(a); h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c); h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e); h[5] = h[5].wrapping_add(f);
        h[6] = h[6].wrapping_add(g); h[7] = h[7].wrapping_add(hh);
    }
    let mut out = String::with_capacity(h.len() * 16);
    for v in &h {
        use std::fmt::Write;
        let _ = write!(out, "{v:016x}");
    }
    out
}

// ─────────────────────────────────────────────────────────────────────────────
// nanoid — URL-safe random ID generator
// ─────────────────────────────────────────────────────────────────────────────

fn verb_nanoid(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    let size = args.first()
        .and_then(|v| match v {
            DynValue::Integer(n) => Some(*n as usize),
            DynValue::Float(n) => Some(*n as usize),
            _ => None,
        })
        .unwrap_or(21)
        .min(256);

    // Check for seed argument for deterministic generation
    let seed = args.get(1).and_then(|v| match v {
        DynValue::String(s) => Some(s.clone()),
        _ => None,
    });

    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789_-";
    let mut result = String::with_capacity(size);

    if let Some(seed_str) = seed {
        // Deterministic: use seed as hash basis (Mulberry32-like)
        let mut state: u32 = 0;
        for b in seed_str.as_bytes() {
            state = state.wrapping_mul(31).wrapping_add(u32::from(*b));
        }
        for _ in 0..size {
            state = state.wrapping_add(0x6D2B_79F5);
            let mut t = state;
            t = (t ^ (t >> 15)).wrapping_mul(t | 1);
            t ^= t.wrapping_add((t ^ (t >> 7)).wrapping_mul(t | 61));
            t ^= t >> 14;
            result.push(ALPHABET[(t as usize) % ALPHABET.len()] as char);
        }
    } else {
        // Non-deterministic: use simple PRNG seeded from address (not crypto-secure)
        let mut state: u64 = 0x1234_5678_9ABC_DEF0;
        // Mix in some entropy from stack address
        state ^= (std::ptr::addr_of!(state) as u64).wrapping_mul(0x517c_c1b7_2722_0a95);
        for _ in 0..size {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            result.push(ALPHABET[(state as usize) % ALPHABET.len()] as char);
        }
    }

    Ok(DynValue::String(result))
}

// ─────────────────────────────────────────────────────────────────────────────
// isValidDate — validate date string against format
// ─────────────────────────────────────────────────────────────────────────────

fn verb_is_valid_date(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    if args.len() < 2 {
        return Err("isValidDate: requires 2 arguments (value, format)".to_string());
    }
    let value = match &args[0] {
        DynValue::String(s) => s.as_str(),
        _ => return Ok(DynValue::Bool(false)),
    };
    let format = match &args[1] {
        DynValue::String(s) => s.as_str(),
        _ => return Ok(DynValue::Bool(false)),
    };

    let valid = match format {
        "YYYY-MM-DD" => parse_validate_date(value, &[0..4, 5..7, 8..10], b'-'),
        "MM/DD/YYYY" => parse_validate_date_mdy(value, '/'),
        "DD/MM/YYYY" => parse_validate_date_dmy(value, '/'),
        "YYYY/MM/DD" => parse_validate_date(value, &[0..4, 5..7, 8..10], b'/'),
        _ => false,
    };

    Ok(DynValue::Bool(valid))
}

fn parse_validate_date(s: &str, ranges: &[std::ops::Range<usize>; 3], sep: u8) -> bool {
    let bytes = s.as_bytes();
    if bytes.len() != 10 || bytes[4] != sep || bytes[7] != sep {
        return false;
    }
    let year: u32 = match s[ranges[0].clone()].parse() { Ok(v) => v, Err(_) => return false };
    let month: u32 = match s[ranges[1].clone()].parse() { Ok(v) => v, Err(_) => return false };
    let day: u32 = match s[ranges[2].clone()].parse() { Ok(v) => v, Err(_) => return false };
    validate_ymd(year, month, day)
}

fn parse_validate_date_mdy(s: &str, sep: char) -> bool {
    let parts: Vec<&str> = s.split(sep).collect();
    if parts.len() != 3 { return false; }
    let month: u32 = match parts[0].parse() { Ok(v) => v, Err(_) => return false };
    let day: u32 = match parts[1].parse() { Ok(v) => v, Err(_) => return false };
    let year: u32 = match parts[2].parse() { Ok(v) => v, Err(_) => return false };
    validate_ymd(year, month, day)
}

fn parse_validate_date_dmy(s: &str, sep: char) -> bool {
    let parts: Vec<&str> = s.split(sep).collect();
    if parts.len() != 3 { return false; }
    let day: u32 = match parts[0].parse() { Ok(v) => v, Err(_) => return false };
    let month: u32 = match parts[1].parse() { Ok(v) => v, Err(_) => return false };
    let year: u32 = match parts[2].parse() { Ok(v) => v, Err(_) => return false };
    validate_ymd(year, month, day)
}

fn validate_ymd(year: u32, month: u32, day: u32) -> bool {
    if !(1..=12).contains(&month) || day < 1 {
        return false;
    }
    let is_leap = (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0);
    let max_day = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => if is_leap { 29 } else { 28 },
        _ => return false,
    };
    day <= max_day
}

/// jsonPath: query nested objects with JSONPath-like expressions.
/// Supports: $.field, [0], [*], ..field (recursive descent)
fn verb_json_path(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    if args.len() < 2 { return Ok(DynValue::Null); }

    let data = match &args[0] {
        DynValue::Object(_) | DynValue::Array(_) => args[0].clone(),
        DynValue::String(s) => {
            // Try to parse as JSON
            match crate::utils::json_parser::parse_json(s) {
                Ok(v) => v,
                Err(_) => return Ok(DynValue::Null),
            }
        }
        _ => return Ok(DynValue::Null),
    };

    let path = match &args[1] {
        DynValue::String(s) => s.clone(),
        _ => return Ok(DynValue::Null),
    };

    // Normalize path
    let mut normalized = path.trim().to_string();
    if normalized.starts_with('$') {
        normalized = normalized[1..].to_string();
    }
    if normalized.starts_with('.') && !normalized.starts_with("..") {
        normalized = normalized[1..].to_string();
    }

    if normalized.is_empty() {
        return Ok(data);
    }

    // Handle recursive descent
    if let Some(after_dots) = normalized.strip_prefix("..") {
        let field = after_dots.split(['.', '[']).next().unwrap_or("");
        let results = json_path_find_recursive(&data, field);
        return Ok(DynValue::Array(results));
    }

    // Parse and traverse path segments
    let segments = json_path_parse_segments(&normalized);
    Ok(json_path_traverse(data, &segments))
}

fn json_path_parse_segments(path: &str) -> Vec<String> {
    let mut segments = Vec::new();
    let mut current = String::new();
    let mut in_bracket = false;

    for ch in path.chars() {
        if ch == '[' && !in_bracket {
            if !current.is_empty() {
                segments.push(current.clone());
                current.clear();
            }
            in_bracket = true;
        } else if ch == ']' && in_bracket {
            segments.push(format!("[{current}]"));
            current.clear();
            in_bracket = false;
        } else if ch == '.' && !in_bracket {
            if !current.is_empty() {
                segments.push(current.clone());
                current.clear();
            }
        } else {
            current.push(ch);
        }
    }
    if !current.is_empty() {
        segments.push(current);
    }
    segments
}

fn json_path_traverse(mut current: DynValue, segments: &[String]) -> DynValue {
    for segment in segments {
        if matches!(current, DynValue::Null) {
            return DynValue::Null;
        }

        // Wildcard [*]
        if segment == "[*]" {
            // Return array as-is (all elements selected)
            continue;
        }

        // Array index [n]
        if segment.starts_with('[') && segment.ends_with(']') {
            let inner = &segment[1..segment.len() - 1];
            if let Ok(idx) = inner.parse::<usize>() {
                current = match current {
                    DynValue::Array(ref arr) => arr.get(idx).cloned().unwrap_or(DynValue::Null),
                    _ => DynValue::Null,
                };
                continue;
            }
        }

        // Object property
        match current {
            DynValue::Object(ref entries) => {
                current = entries.iter()
                    .find(|(k, _)| k == segment)
                    .map_or(DynValue::Null, |(_, v)| v.clone());
            }
            DynValue::Array(ref items) => {
                // Map over array elements
                let mapped: Vec<DynValue> = items.iter().filter_map(|item| {
                    if let DynValue::Object(entries) = item {
                        entries.iter().find(|(k, _)| k == segment).map(|(_, v)| v.clone())
                    } else {
                        None
                    }
                }).collect();
                current = DynValue::Array(mapped);
            }
            _ => return DynValue::Null,
        }
    }
    current
}

fn json_path_find_recursive(data: &DynValue, field: &str) -> Vec<DynValue> {
    let mut results = Vec::new();
    match data {
        DynValue::Object(entries) => {
            for (k, v) in entries {
                if k == field {
                    results.push(v.clone());
                }
                results.extend(json_path_find_recursive(v, field));
            }
        }
        DynValue::Array(items) => {
            for item in items {
                results.extend(json_path_find_recursive(item, field));
            }
        }
        _ => {}
    }
    results
}

/// formatLocaleDate: format a date using locale-specific conventions.
/// Args: (date, [locale])
/// In Rust we implement common locale patterns directly (no Intl.DateTimeFormat).
fn verb_format_locale_date(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    if args.is_empty() { return Ok(DynValue::Null); }

    let date_str = match &args[0] {
        DynValue::String(s) => s.clone(),
        _ => return Ok(DynValue::Null),
    };

    // Parse YYYY-MM-DD
    let parts: Vec<&str> = date_str.split('-').collect();
    if parts.len() < 3 { return Ok(DynValue::Null); }
    let year: u32 = match parts[0].parse() { Ok(v) => v, Err(_) => return Ok(DynValue::Null) };
    let month: u32 = match parts[1].parse() { Ok(v) => v, Err(_) => return Ok(DynValue::Null) };
    let day: u32 = match parts[2].split('T').next().unwrap_or("").parse() { Ok(v) => v, Err(_) => return Ok(DynValue::Null) };

    let locale = if args.len() >= 2 {
        match &args[1] { DynValue::String(s) => s.to_lowercase(), _ => "en-us".to_string() }
    } else {
        "en-us".to_string()
    };

    // Format based on locale pattern
    let formatted = match locale.as_str() {
        "en-gb" | "en_gb"
        | "fr-fr" | "fr_fr" | "fr"
        | "it-it" | "it_it" | "it"
        | "es-es" | "es_es" | "es"
        | "pt-br" | "pt_br" | "pt" => format!("{day}/{month}/{year}"),
        "de-de" | "de_de" | "de" => format!("{day}.{month}.{year}"),
        "ja-jp" | "ja_jp" | "ja" | "zh-cn" | "zh_cn" | "zh" => format!("{year}/{month}/{day}"),
        "ko-kr" | "ko_kr" | "ko" => format!("{year}.{month}.{day}"),
        "nl-nl" | "nl_nl" | "nl" => format!("{day}-{month}-{year}"),
        "ru-ru" | "ru_ru" | "ru" => format!("{day}.{month:02}.{year}"),
        _ => format!("{month}/{day}/{year}"), // default to US-style (en-us)
    };

    Ok(DynValue::String(formatted))
}

// =============================================================================
// Extended tests for verb parity with TypeScript SDK
// =============================================================================

#[cfg(test)]
mod extended_tests {
    use super::*;
    use std::collections::HashMap;

    fn ctx() -> VerbContext {
        VerbContext {
            source: DynValue::Null,
            loop_vars: HashMap::new(),
            accumulators: HashMap::new(),
            tables: HashMap::new(),
        }
    }

    fn s(v: &str) -> DynValue { DynValue::String(v.to_string()) }
    fn i(v: i64) -> DynValue { DynValue::Integer(v) }
    fn f(v: f64) -> DynValue { DynValue::Float(v) }
    fn b(v: bool) -> DynValue { DynValue::Bool(v) }
    fn null() -> DynValue { DynValue::Null }
    fn arr(items: Vec<DynValue>) -> DynValue { DynValue::Array(items) }
    fn obj(pairs: Vec<(&str, DynValue)>) -> DynValue {
        let entries: Vec<(String, DynValue)> = pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect();
        DynValue::Object(entries)
    }

    // =========================================================================
    // Logic verbs
    // =========================================================================

    // --- and ---
    #[test] fn and_true_true() { assert_eq!(verb_and(&[b(true), b(true)], &ctx()).unwrap(), b(true)); }
    #[test] fn and_true_false() { assert_eq!(verb_and(&[b(true), b(false)], &ctx()).unwrap(), b(false)); }
    #[test] fn and_false_true() { assert_eq!(verb_and(&[b(false), b(true)], &ctx()).unwrap(), b(false)); }
    #[test] fn and_false_false() { assert_eq!(verb_and(&[b(false), b(false)], &ctx()).unwrap(), b(false)); }
    #[test] fn and_too_few_args() { assert!(verb_and(&[b(true)], &ctx()).is_err()); }
    #[test] fn and_non_bool() { assert!(verb_and(&[i(1), i(0)], &ctx()).is_err()); }

    // --- or ---
    #[test] fn or_true_true() { assert_eq!(verb_or(&[b(true), b(true)], &ctx()).unwrap(), b(true)); }
    #[test] fn or_true_false() { assert_eq!(verb_or(&[b(true), b(false)], &ctx()).unwrap(), b(true)); }
    #[test] fn or_false_true() { assert_eq!(verb_or(&[b(false), b(true)], &ctx()).unwrap(), b(true)); }
    #[test] fn or_false_false() { assert_eq!(verb_or(&[b(false), b(false)], &ctx()).unwrap(), b(false)); }
    #[test] fn or_too_few_args() { assert!(verb_or(&[b(true)], &ctx()).is_err()); }

    // --- not ---
    #[test] fn not_true() { assert_eq!(verb_not(&[b(true)], &ctx()).unwrap(), b(false)); }
    #[test] fn not_false() { assert_eq!(verb_not(&[b(false)], &ctx()).unwrap(), b(true)); }
    #[test] fn not_null() { assert_eq!(verb_not(&[null()], &ctx()).unwrap(), b(true)); }
    #[test] fn not_zero_int() { assert_eq!(verb_not(&[i(0)], &ctx()).unwrap(), b(true)); }
    #[test] fn not_nonzero_int() { assert_eq!(verb_not(&[i(5)], &ctx()).unwrap(), b(false)); }
    #[test] fn not_zero_float() { assert_eq!(verb_not(&[f(0.0)], &ctx()).unwrap(), b(true)); }
    #[test] fn not_empty_string() { assert_eq!(verb_not(&[s("")], &ctx()).unwrap(), b(true)); }
    #[test] fn not_false_string() { assert_eq!(verb_not(&[s("false")], &ctx()).unwrap(), b(true)); }
    #[test] fn not_nonempty_string() { assert_eq!(verb_not(&[s("hello")], &ctx()).unwrap(), b(false)); }
    #[test] fn not_empty_array() { assert_eq!(verb_not(&[arr(vec![])], &ctx()).unwrap(), b(true)); }
    #[test] fn not_nonempty_array() { assert_eq!(verb_not(&[arr(vec![i(1)])], &ctx()).unwrap(), b(false)); }

    // --- xor ---
    #[test] fn xor_true_true() { assert_eq!(verb_xor(&[b(true), b(true)], &ctx()).unwrap(), b(false)); }
    #[test] fn xor_true_false() { assert_eq!(verb_xor(&[b(true), b(false)], &ctx()).unwrap(), b(true)); }
    #[test] fn xor_false_true() { assert_eq!(verb_xor(&[b(false), b(true)], &ctx()).unwrap(), b(true)); }
    #[test] fn xor_false_false() { assert_eq!(verb_xor(&[b(false), b(false)], &ctx()).unwrap(), b(false)); }
    #[test] fn xor_too_few_args() { assert!(verb_xor(&[b(true)], &ctx()).is_err()); }
    #[test] fn xor_non_bool() { assert!(verb_xor(&[i(1), i(0)], &ctx()).is_err()); }

    // --- eq ---
    #[test] fn eq_ints_equal() { assert_eq!(verb_eq(&[i(5), i(5)], &ctx()).unwrap(), b(true)); }
    #[test] fn eq_ints_not_equal() { assert_eq!(verb_eq(&[i(5), i(6)], &ctx()).unwrap(), b(false)); }
    #[test] fn eq_strings_equal() { assert_eq!(verb_eq(&[s("abc"), s("abc")], &ctx()).unwrap(), b(true)); }
    #[test] fn eq_strings_not_equal() { assert_eq!(verb_eq(&[s("abc"), s("xyz")], &ctx()).unwrap(), b(false)); }
    #[test] fn eq_int_float_cross() { assert_eq!(verb_eq(&[i(5), f(5.0)], &ctx()).unwrap(), b(true)); }
    #[test] fn eq_string_int_coercion() { assert_eq!(verb_eq(&[s("42"), i(42)], &ctx()).unwrap(), b(true)); }
    #[test] fn eq_nulls() { assert_eq!(verb_eq(&[null(), null()], &ctx()).unwrap(), b(true)); }
    #[test] fn eq_null_vs_string() { assert_eq!(verb_eq(&[null(), s("")], &ctx()).unwrap(), b(false)); }
    #[test] fn eq_bools() { assert_eq!(verb_eq(&[b(true), b(true)], &ctx()).unwrap(), b(true)); }
    #[test] fn eq_too_few() { assert!(verb_eq(&[i(1)], &ctx()).is_err()); }

    // --- ne ---
    #[test] fn ne_equal() { assert_eq!(verb_ne(&[i(5), i(5)], &ctx()).unwrap(), b(false)); }
    #[test] fn ne_not_equal() { assert_eq!(verb_ne(&[i(5), i(6)], &ctx()).unwrap(), b(true)); }
    #[test] fn ne_cross_type() { assert_eq!(verb_ne(&[i(5), f(5.0)], &ctx()).unwrap(), b(false)); }
    #[test] fn ne_too_few() { assert!(verb_ne(&[i(1)], &ctx()).is_err()); }

    // --- lt ---
    #[test] fn lt_ints_less() { assert_eq!(verb_lt(&[i(3), i(5)], &ctx()).unwrap(), b(true)); }
    #[test] fn lt_ints_equal() { assert_eq!(verb_lt(&[i(5), i(5)], &ctx()).unwrap(), b(false)); }
    #[test] fn lt_ints_greater() { assert_eq!(verb_lt(&[i(7), i(5)], &ctx()).unwrap(), b(false)); }
    #[test] fn lt_floats() { assert_eq!(verb_lt(&[f(1.5), f(2.5)], &ctx()).unwrap(), b(true)); }
    #[test] fn lt_strings() { assert_eq!(verb_lt(&[s("abc"), s("xyz")], &ctx()).unwrap(), b(true)); }
    #[test] fn lt_too_few() { assert!(verb_lt(&[i(1)], &ctx()).is_err()); }

    // --- lte ---
    #[test] fn lte_less() { assert_eq!(verb_lte(&[i(3), i(5)], &ctx()).unwrap(), b(true)); }
    #[test] fn lte_equal() { assert_eq!(verb_lte(&[i(5), i(5)], &ctx()).unwrap(), b(true)); }
    #[test] fn lte_greater() { assert_eq!(verb_lte(&[i(7), i(5)], &ctx()).unwrap(), b(false)); }
    #[test] fn lte_strings() { assert_eq!(verb_lte(&[s("abc"), s("abc")], &ctx()).unwrap(), b(true)); }

    // --- gt ---
    #[test] fn gt_greater() { assert_eq!(verb_gt(&[i(7), i(5)], &ctx()).unwrap(), b(true)); }
    #[test] fn gt_equal() { assert_eq!(verb_gt(&[i(5), i(5)], &ctx()).unwrap(), b(false)); }
    #[test] fn gt_less() { assert_eq!(verb_gt(&[i(3), i(5)], &ctx()).unwrap(), b(false)); }
    #[test] fn gt_strings() { assert_eq!(verb_gt(&[s("xyz"), s("abc")], &ctx()).unwrap(), b(true)); }

    // --- gte ---
    #[test] fn gte_greater() { assert_eq!(verb_gte(&[i(7), i(5)], &ctx()).unwrap(), b(true)); }
    #[test] fn gte_equal() { assert_eq!(verb_gte(&[i(5), i(5)], &ctx()).unwrap(), b(true)); }
    #[test] fn gte_less() { assert_eq!(verb_gte(&[i(3), i(5)], &ctx()).unwrap(), b(false)); }

    // --- between ---
    #[test] fn between_in_range() { assert_eq!(verb_between(&[i(5), i(1), i(10)], &ctx()).unwrap(), b(true)); }
    #[test] fn between_at_min() { assert_eq!(verb_between(&[i(1), i(1), i(10)], &ctx()).unwrap(), b(true)); }
    #[test] fn between_at_max() { assert_eq!(verb_between(&[i(10), i(1), i(10)], &ctx()).unwrap(), b(true)); }
    #[test] fn between_below() { assert_eq!(verb_between(&[i(0), i(1), i(10)], &ctx()).unwrap(), b(false)); }
    #[test] fn between_above() { assert_eq!(verb_between(&[i(11), i(1), i(10)], &ctx()).unwrap(), b(false)); }
    #[test] fn between_floats() { assert_eq!(verb_between(&[f(5.5), f(1.0), f(10.0)], &ctx()).unwrap(), b(true)); }
    #[test] fn between_strings() { assert_eq!(verb_between(&[s("dog"), s("cat"), s("fox")], &ctx()).unwrap(), b(true)); }
    #[test] fn between_too_few() { assert!(verb_between(&[i(5), i(1)], &ctx()).is_err()); }

    // --- cond ---
    #[test] fn cond_first_true() { assert_eq!(verb_cond(&[b(true), s("yes"), b(false), s("no")], &ctx()).unwrap(), s("yes")); }
    #[test] fn cond_second_true() { assert_eq!(verb_cond(&[b(false), s("yes"), b(true), s("no")], &ctx()).unwrap(), s("no")); }
    #[test] fn cond_default() { assert_eq!(verb_cond(&[b(false), s("yes"), s("default")], &ctx()).unwrap(), s("default")); }
    #[test] fn cond_no_match() { assert_eq!(verb_cond(&[b(false), s("yes"), b(false), s("no")], &ctx()).unwrap(), null()); }
    #[test] fn cond_empty() { assert_eq!(verb_cond(&[], &ctx()).unwrap(), null()); }

    // --- ifElse ---
    #[test] fn if_else_true() { assert_eq!(verb_if_else(&[b(true), s("yes"), s("no")], &ctx()).unwrap(), s("yes")); }
    #[test] fn if_else_false() { assert_eq!(verb_if_else(&[b(false), s("yes"), s("no")], &ctx()).unwrap(), s("no")); }
    #[test] fn if_else_truthy_int() { assert_eq!(verb_if_else(&[i(1), s("yes"), s("no")], &ctx()).unwrap(), s("yes")); }
    #[test] fn if_else_falsy_zero() { assert_eq!(verb_if_else(&[i(0), s("yes"), s("no")], &ctx()).unwrap(), s("no")); }
    #[test] fn if_else_null_is_falsy() { assert_eq!(verb_if_else(&[null(), s("yes"), s("no")], &ctx()).unwrap(), s("no")); }
    #[test] fn if_else_too_few() { assert!(verb_if_else(&[b(true), s("yes")], &ctx()).is_err()); }

    // --- ifNull ---
    #[test] fn if_null_not_null() { assert_eq!(verb_if_null(&[s("val"), s("default")], &ctx()).unwrap(), s("val")); }
    #[test] fn if_null_is_null() { assert_eq!(verb_if_null(&[null(), s("default")], &ctx()).unwrap(), s("default")); }
    #[test] fn if_null_too_few() { assert!(verb_if_null(&[null()], &ctx()).is_err()); }

    // --- ifEmpty ---
    #[test] fn if_empty_not_empty() { assert_eq!(verb_if_empty(&[s("val"), s("default")], &ctx()).unwrap(), s("val")); }
    #[test] fn if_empty_empty_string() { assert_eq!(verb_if_empty(&[s(""), s("default")], &ctx()).unwrap(), s("default")); }
    #[test] fn if_empty_null() { assert_eq!(verb_if_empty(&[null(), s("default")], &ctx()).unwrap(), s("default")); }
    #[test] fn if_empty_int_not_empty() { assert_eq!(verb_if_empty(&[i(0), s("default")], &ctx()).unwrap(), i(0)); }
    #[test] fn if_empty_too_few() { assert!(verb_if_empty(&[s("")], &ctx()).is_err()); }

    // --- switch ---
    #[test] fn switch_match_first() {
        assert_eq!(numeric_verbs::switch_verb(&[s("a"), s("a"), s("alpha"), s("b"), s("beta")], &ctx()).unwrap(), s("alpha"));
    }
    #[test] fn switch_match_second() {
        assert_eq!(numeric_verbs::switch_verb(&[s("b"), s("a"), s("alpha"), s("b"), s("beta")], &ctx()).unwrap(), s("beta"));
    }
    #[test] fn switch_default() {
        assert_eq!(numeric_verbs::switch_verb(&[s("c"), s("a"), s("alpha"), s("default")], &ctx()).unwrap(), s("default"));
    }
    #[test] fn switch_no_match_no_default() {
        assert_eq!(numeric_verbs::switch_verb(&[s("c"), s("a"), s("alpha"), s("b"), s("beta")], &ctx()).unwrap(), null());
    }
    #[test] fn switch_empty() { assert!(numeric_verbs::switch_verb(&[], &ctx()).is_err()); }

    // =========================================================================
    // Type checking verbs
    // =========================================================================

    // --- isNull ---
    #[test] fn is_null_null() { assert_eq!(verb_is_null(&[null()], &ctx()).unwrap(), b(true)); }
    #[test] fn is_null_string() { assert_eq!(verb_is_null(&[s("hi")], &ctx()).unwrap(), b(false)); }
    #[test] fn is_null_int() { assert_eq!(verb_is_null(&[i(0)], &ctx()).unwrap(), b(false)); }
    #[test] fn is_null_empty_args() { assert_eq!(verb_is_null(&[], &ctx()).unwrap(), b(true)); }

    // --- isString ---
    #[test] fn is_string_string() { assert_eq!(verb_is_string(&[s("hello")], &ctx()).unwrap(), b(true)); }
    #[test] fn is_string_empty() { assert_eq!(verb_is_string(&[s("")], &ctx()).unwrap(), b(true)); }
    #[test] fn is_string_int() { assert_eq!(verb_is_string(&[i(5)], &ctx()).unwrap(), b(false)); }
    #[test] fn is_string_null() { assert_eq!(verb_is_string(&[null()], &ctx()).unwrap(), b(false)); }
    #[test] fn is_string_no_args() { assert_eq!(verb_is_string(&[], &ctx()).unwrap(), b(false)); }

    // --- isNumber ---
    #[test] fn is_number_int() { assert_eq!(verb_is_number(&[i(42)], &ctx()).unwrap(), b(true)); }
    #[test] fn is_number_float() { assert_eq!(verb_is_number(&[f(3.14)], &ctx()).unwrap(), b(true)); }
    #[test] fn is_number_string() { assert_eq!(verb_is_number(&[s("42")], &ctx()).unwrap(), b(false)); }
    #[test] fn is_number_null() { assert_eq!(verb_is_number(&[null()], &ctx()).unwrap(), b(false)); }
    #[test] fn is_number_bool() { assert_eq!(verb_is_number(&[b(true)], &ctx()).unwrap(), b(false)); }
    #[test] fn is_number_no_args() { assert_eq!(verb_is_number(&[], &ctx()).unwrap(), b(false)); }

    // --- isBoolean ---
    #[test] fn is_boolean_true() { assert_eq!(verb_is_boolean(&[b(true)], &ctx()).unwrap(), b(true)); }
    #[test] fn is_boolean_false() { assert_eq!(verb_is_boolean(&[b(false)], &ctx()).unwrap(), b(true)); }
    #[test] fn is_boolean_string() { assert_eq!(verb_is_boolean(&[s("true")], &ctx()).unwrap(), b(false)); }
    #[test] fn is_boolean_no_args() { assert_eq!(verb_is_boolean(&[], &ctx()).unwrap(), b(false)); }

    // --- isArray ---
    #[test] fn is_array_arr() { assert_eq!(verb_is_array(&[arr(vec![i(1)])], &ctx()).unwrap(), b(true)); }
    #[test] fn is_array_empty() { assert_eq!(verb_is_array(&[arr(vec![])], &ctx()).unwrap(), b(true)); }
    #[test] fn is_array_string_like() { assert_eq!(verb_is_array(&[s("[1,2]")], &ctx()).unwrap(), b(true)); }
    #[test] fn is_array_int() { assert_eq!(verb_is_array(&[i(5)], &ctx()).unwrap(), b(false)); }
    #[test] fn is_array_no_args() { assert_eq!(verb_is_array(&[], &ctx()).unwrap(), b(false)); }

    // --- isObject ---
    #[test] fn is_object_obj() { assert_eq!(verb_is_object(&[obj(vec![("a", i(1))])], &ctx()).unwrap(), b(true)); }
    #[test] fn is_object_string_like() { assert_eq!(verb_is_object(&[s("{}")], &ctx()).unwrap(), b(true)); }
    #[test] fn is_object_arr() { assert_eq!(verb_is_object(&[arr(vec![])], &ctx()).unwrap(), b(false)); }
    #[test] fn is_object_no_args() { assert_eq!(verb_is_object(&[], &ctx()).unwrap(), b(false)); }

    // --- isDate ---
    #[test] fn is_date_valid() { assert_eq!(verb_is_date(&[s("2024-01-15")], &ctx()).unwrap(), b(true)); }
    #[test] fn is_date_timestamp() { assert_eq!(verb_is_date(&[s("2024-01-15T10:30:00")], &ctx()).unwrap(), b(true)); }
    #[test] fn is_date_invalid() { assert_eq!(verb_is_date(&[s("not-a-date")], &ctx()).unwrap(), b(false)); }
    #[test] fn is_date_int() { assert_eq!(verb_is_date(&[i(20240115)], &ctx()).unwrap(), b(false)); }
    #[test] fn is_date_null() { assert_eq!(verb_is_date(&[null()], &ctx()).unwrap(), b(false)); }

    // --- typeOf ---
    #[test] fn type_of_null() { assert_eq!(verb_type_of(&[null()], &ctx()).unwrap(), s("null")); }
    #[test] fn type_of_bool() { assert_eq!(verb_type_of(&[b(true)], &ctx()).unwrap(), s("boolean")); }
    #[test] fn type_of_string() { assert_eq!(verb_type_of(&[s("hi")], &ctx()).unwrap(), s("string")); }
    #[test] fn type_of_integer() { assert_eq!(verb_type_of(&[i(42)], &ctx()).unwrap(), s("integer")); }
    #[test] fn type_of_float() { assert_eq!(verb_type_of(&[f(3.14)], &ctx()).unwrap(), s("number")); }
    #[test] fn type_of_array() { assert_eq!(verb_type_of(&[arr(vec![])], &ctx()).unwrap(), s("array")); }
    #[test] fn type_of_object() { assert_eq!(verb_type_of(&[obj(vec![])], &ctx()).unwrap(), s("object")); }
    #[test] fn type_of_no_args() { assert_eq!(verb_type_of(&[], &ctx()).unwrap(), s("null")); }

    // --- isFinite ---
    #[test] fn is_finite_normal() { assert_eq!(numeric_verbs::is_finite(&[f(42.0)], &ctx()).unwrap(), b(true)); }
    #[test] fn is_finite_inf() { assert_eq!(numeric_verbs::is_finite(&[f(f64::INFINITY)], &ctx()).unwrap(), b(false)); }
    #[test] fn is_finite_neg_inf() { assert_eq!(numeric_verbs::is_finite(&[f(f64::NEG_INFINITY)], &ctx()).unwrap(), b(false)); }
    #[test] fn is_finite_nan() { assert_eq!(numeric_verbs::is_finite(&[f(f64::NAN)], &ctx()).unwrap(), b(false)); }
    #[test] fn is_finite_int() { assert_eq!(numeric_verbs::is_finite(&[i(100)], &ctx()).unwrap(), b(true)); }

    // --- isNaN ---
    #[test] fn is_nan_nan() { assert_eq!(numeric_verbs::is_nan(&[f(f64::NAN)], &ctx()).unwrap(), b(true)); }
    #[test] fn is_nan_normal() { assert_eq!(numeric_verbs::is_nan(&[f(42.0)], &ctx()).unwrap(), b(false)); }
    #[test] fn is_nan_int() { assert_eq!(numeric_verbs::is_nan(&[i(0)], &ctx()).unwrap(), b(false)); }

    // =========================================================================
    // Coercion verbs
    // =========================================================================

    // --- coerceString ---
    #[test] fn coerce_str_from_string() { assert_eq!(verb_coerce_string(&[s("hi")], &ctx()).unwrap(), s("hi")); }
    #[test] fn coerce_str_from_int() { assert_eq!(verb_coerce_string(&[i(42)], &ctx()).unwrap(), s("42")); }
    #[test] fn coerce_str_from_float() { assert_eq!(verb_coerce_string(&[f(3.14)], &ctx()).unwrap(), s("3.14")); }
    #[test] fn coerce_str_from_bool() { assert_eq!(verb_coerce_string(&[b(true)], &ctx()).unwrap(), s("true")); }
    #[test] fn coerce_str_from_null() { assert_eq!(verb_coerce_string(&[null()], &ctx()).unwrap(), null()); }

    // --- coerceNumber ---
    #[test] fn coerce_num_from_int() { assert_eq!(verb_coerce_number(&[i(42)], &ctx()).unwrap(), f(42.0)); }
    #[test] fn coerce_num_from_float() { assert_eq!(verb_coerce_number(&[f(3.14)], &ctx()).unwrap(), f(3.14)); }
    #[test] fn coerce_num_from_string() { assert_eq!(verb_coerce_number(&[s("3.14")], &ctx()).unwrap(), f(3.14)); }
    #[test] fn coerce_num_from_bool_true() { assert_eq!(verb_coerce_number(&[b(true)], &ctx()).unwrap(), f(1.0)); }
    #[test] fn coerce_num_from_bool_false() { assert_eq!(verb_coerce_number(&[b(false)], &ctx()).unwrap(), f(0.0)); }
    #[test] fn coerce_num_from_null() { assert_eq!(verb_coerce_number(&[null()], &ctx()).unwrap(), null()); }
    #[test] fn coerce_num_invalid_string() { assert!(verb_coerce_number(&[s("abc")], &ctx()).is_err()); }

    // --- coerceBoolean ---
    #[test] fn coerce_bool_from_true() { assert_eq!(verb_coerce_boolean(&[b(true)], &ctx()).unwrap(), b(true)); }
    #[test] fn coerce_bool_from_false() { assert_eq!(verb_coerce_boolean(&[b(false)], &ctx()).unwrap(), b(false)); }
    #[test] fn coerce_bool_from_string_true() { assert_eq!(verb_coerce_boolean(&[s("true")], &ctx()).unwrap(), b(true)); }
    #[test] fn coerce_bool_from_string_false() { assert_eq!(verb_coerce_boolean(&[s("false")], &ctx()).unwrap(), b(false)); }
    #[test] fn coerce_bool_from_string_zero() { assert_eq!(verb_coerce_boolean(&[s("0")], &ctx()).unwrap(), b(false)); }
    #[test] fn coerce_bool_from_string_empty() { assert_eq!(verb_coerce_boolean(&[s("")], &ctx()).unwrap(), b(false)); }
    #[test] fn coerce_bool_from_string_no() { assert_eq!(verb_coerce_boolean(&[s("no")], &ctx()).unwrap(), b(false)); }
    #[test] fn coerce_bool_from_string_yes() { assert_eq!(verb_coerce_boolean(&[s("yes")], &ctx()).unwrap(), b(true)); }
    #[test] fn coerce_bool_from_int_nonzero() { assert_eq!(verb_coerce_boolean(&[i(1)], &ctx()).unwrap(), b(true)); }
    #[test] fn coerce_bool_from_int_zero() { assert_eq!(verb_coerce_boolean(&[i(0)], &ctx()).unwrap(), b(false)); }
    #[test] fn coerce_bool_from_null() { assert_eq!(verb_coerce_boolean(&[null()], &ctx()).unwrap(), b(false)); }

    // --- coerceInteger ---
    #[test] fn coerce_int_from_int() { assert_eq!(verb_coerce_integer(&[i(42)], &ctx()).unwrap(), i(42)); }
    #[test] fn coerce_int_from_float() { assert_eq!(verb_coerce_integer(&[f(3.7)], &ctx()).unwrap(), i(3)); }
    #[test] fn coerce_int_from_string() { assert_eq!(verb_coerce_integer(&[s("42")], &ctx()).unwrap(), i(42)); }
    #[test] fn coerce_int_from_string_float() { assert_eq!(verb_coerce_integer(&[s("3.9")], &ctx()).unwrap(), i(3)); }
    #[test] fn coerce_int_from_bool() { assert_eq!(verb_coerce_integer(&[b(true)], &ctx()).unwrap(), i(1)); }
    #[test] fn coerce_int_from_null() { assert_eq!(verb_coerce_integer(&[null()], &ctx()).unwrap(), null()); }
    #[test] fn coerce_int_invalid() { assert!(verb_coerce_integer(&[s("abc")], &ctx()).is_err()); }

    // --- coerceDate ---
    #[test] fn coerce_date_valid() { assert_eq!(verb_coerce_date(&[s("2024-01-15")], &ctx()).unwrap(), DynValue::Date("2024-01-15".to_string())); }
    #[test] fn coerce_date_timestamp() { assert_eq!(verb_coerce_date(&[s("2024-01-15T10:30:00")], &ctx()).unwrap(), DynValue::Date("2024-01-15".to_string())); }
    #[test] fn coerce_date_invalid() { assert!(verb_coerce_date(&[s("not-a-date")], &ctx()).is_err()); }
    #[test] fn coerce_date_null() { assert_eq!(verb_coerce_date(&[null()], &ctx()).unwrap(), null()); }

    // --- coerceTimestamp ---
    #[test] fn coerce_ts_valid() { assert_eq!(verb_coerce_timestamp(&[s("2024-01-15T10:30:00")], &ctx()).unwrap(), s("2024-01-15T10:30:00")); }
    #[test] fn coerce_ts_date_only() { assert_eq!(verb_coerce_timestamp(&[s("2024-01-15")], &ctx()).unwrap(), s("2024-01-15T00:00:00")); }
    #[test] fn coerce_ts_invalid() { assert!(verb_coerce_timestamp(&[s("not-valid")], &ctx()).is_err()); }
    #[test] fn coerce_ts_null() { assert_eq!(verb_coerce_timestamp(&[null()], &ctx()).unwrap(), null()); }

    // --- tryCoerce ---
    #[test] fn try_coerce_integer() { assert_eq!(verb_try_coerce(&[s("42")], &ctx()).unwrap(), i(42)); }
    #[test] fn try_coerce_float() { assert_eq!(verb_try_coerce(&[s("3.14")], &ctx()).unwrap(), f(3.14)); }
    #[test] fn try_coerce_bool_true() { assert_eq!(verb_try_coerce(&[s("true")], &ctx()).unwrap(), b(true)); }
    #[test] fn try_coerce_bool_false() { assert_eq!(verb_try_coerce(&[s("false")], &ctx()).unwrap(), b(false)); }
    #[test] fn try_coerce_date() {
        let result = verb_try_coerce(&[s("2024-01-15")], &ctx()).unwrap();
        assert_eq!(result, DynValue::Date("2024-01-15".to_string()));
    }
    #[test] fn try_coerce_plain_string() { assert_eq!(verb_try_coerce(&[s("hello")], &ctx()).unwrap(), s("hello")); }
    #[test] fn try_coerce_no_args() { assert_eq!(verb_try_coerce(&[], &ctx()).unwrap(), null()); }
    #[test] fn try_coerce_passthrough_int() { assert_eq!(verb_try_coerce(&[i(99)], &ctx()).unwrap(), i(99)); }

    // --- toArray ---
    #[test] fn to_array_from_array() { assert_eq!(verb_to_array(&[arr(vec![i(1), i(2)])], &ctx()).unwrap(), arr(vec![i(1), i(2)])); }
    #[test] fn to_array_from_scalar() { assert_eq!(verb_to_array(&[i(42)], &ctx()).unwrap(), arr(vec![i(42)])); }
    #[test] fn to_array_no_args() { assert_eq!(verb_to_array(&[], &ctx()).unwrap(), arr(vec![])); }

    // --- toObject ---
    #[test] fn to_object_from_pairs() {
        let input = arr(vec![
            arr(vec![s("a"), i(1)]),
            arr(vec![s("b"), i(2)]),
        ]);
        let result = verb_to_object(&[input], &ctx()).unwrap();
        if let DynValue::Object(entries) = result {
            assert_eq!(entries.len(), 2);
            assert_eq!(entries[0], ("a".to_string(), i(1)));
        } else { panic!("expected object"); }
    }
    #[test] fn to_object_null() { assert_eq!(verb_to_object(&[null()], &ctx()).unwrap(), null()); }

    // =========================================================================
    // Core string verbs
    // =========================================================================

    // --- concat ---
    #[test] fn concat_strings() { assert_eq!(verb_concat(&[s("hello"), s(" "), s("world")], &ctx()).unwrap(), s("hello world")); }
    #[test] fn concat_mixed() { assert_eq!(verb_concat(&[s("val="), i(42)], &ctx()).unwrap(), s("val=42")); }
    #[test] fn concat_with_null() { assert_eq!(verb_concat(&[s("a"), null(), s("b")], &ctx()).unwrap(), s("ab")); }
    #[test] fn concat_empty() { assert_eq!(verb_concat(&[], &ctx()).unwrap(), s("")); }
    #[test] fn concat_bool() { assert_eq!(verb_concat(&[s("is: "), b(true)], &ctx()).unwrap(), s("is: true")); }

    // --- upper ---
    #[test] fn upper_basic() { assert_eq!(verb_upper(&[s("hello")], &ctx()).unwrap(), s("HELLO")); }
    #[test] fn upper_already() { assert_eq!(verb_upper(&[s("HELLO")], &ctx()).unwrap(), s("HELLO")); }
    #[test] fn upper_empty() { assert_eq!(verb_upper(&[s("")], &ctx()).unwrap(), s("")); }
    #[test] fn upper_null() { assert_eq!(verb_upper(&[null()], &ctx()).unwrap(), null()); }
    #[test] fn upper_unicode() { assert_eq!(verb_upper(&[s("caf\u{00e9}")], &ctx()).unwrap(), s("CAF\u{00c9}")); }

    // --- lower ---
    #[test] fn lower_basic() { assert_eq!(verb_lower(&[s("HELLO")], &ctx()).unwrap(), s("hello")); }
    #[test] fn lower_null() { assert_eq!(verb_lower(&[null()], &ctx()).unwrap(), null()); }
    #[test] fn lower_empty() { assert_eq!(verb_lower(&[s("")], &ctx()).unwrap(), s("")); }

    // --- trim ---
    #[test] fn trim_basic() { assert_eq!(verb_trim(&[s("  hello  ")], &ctx()).unwrap(), s("hello")); }
    #[test] fn trim_null() { assert_eq!(verb_trim(&[null()], &ctx()).unwrap(), null()); }
    #[test] fn trim_no_spaces() { assert_eq!(verb_trim(&[s("hello")], &ctx()).unwrap(), s("hello")); }
    #[test] fn trim_tabs() { assert_eq!(verb_trim(&[s("\thello\t")], &ctx()).unwrap(), s("hello")); }

    // --- trimLeft ---
    #[test] fn trim_left_basic() { assert_eq!(verb_trim_left(&[s("  hello  ")], &ctx()).unwrap(), s("hello  ")); }
    #[test] fn trim_left_null() { assert_eq!(verb_trim_left(&[null()], &ctx()).unwrap(), null()); }

    // --- trimRight ---
    #[test] fn trim_right_basic() { assert_eq!(verb_trim_right(&[s("  hello  ")], &ctx()).unwrap(), s("  hello")); }
    #[test] fn trim_right_null() { assert_eq!(verb_trim_right(&[null()], &ctx()).unwrap(), null()); }

    // --- capitalize ---
    #[test] fn capitalize_basic() { assert_eq!(verb_capitalize(&[s("hello")], &ctx()).unwrap(), s("Hello")); }
    #[test] fn capitalize_empty() { assert_eq!(verb_capitalize(&[s("")], &ctx()).unwrap(), s("")); }
    #[test] fn capitalize_null() { assert_eq!(verb_capitalize(&[null()], &ctx()).unwrap(), null()); }
    #[test] fn capitalize_already() { assert_eq!(verb_capitalize(&[s("Hello")], &ctx()).unwrap(), s("Hello")); }
    #[test] fn capitalize_single_char() { assert_eq!(verb_capitalize(&[s("h")], &ctx()).unwrap(), s("H")); }

    // --- replace ---
    #[test] fn replace_basic() { assert_eq!(verb_replace(&[s("hello world"), s("world"), s("rust")], &ctx()).unwrap(), s("hello rust")); }
    #[test] fn replace_no_match() { assert_eq!(verb_replace(&[s("hello"), s("xyz"), s("abc")], &ctx()).unwrap(), s("hello")); }
    #[test] fn replace_multiple() { assert_eq!(verb_replace(&[s("aaa"), s("a"), s("b")], &ctx()).unwrap(), s("bbb")); }
    #[test] fn replace_too_few() { assert!(verb_replace(&[s("hello"), s("h")], &ctx()).is_err()); }

    // --- substring ---
    #[test] fn substring_basic() { assert_eq!(verb_substring(&[s("hello"), i(1), i(4)], &ctx()).unwrap(), s("ell")); }
    #[test] fn substring_no_end() { assert_eq!(verb_substring(&[s("hello"), i(2)], &ctx()).unwrap(), s("llo")); }
    #[test] fn substring_start_beyond() { assert_eq!(verb_substring(&[s("hello"), i(10)], &ctx()).unwrap(), s("")); }
    #[test] fn substring_start_negative() { assert_eq!(verb_substring(&[s("hello"), i(-1)], &ctx()).unwrap(), s("hello")); }
    #[test] fn substring_too_few() { assert!(verb_substring(&[s("hello")], &ctx()).is_err()); }

    // --- length ---
    #[test] fn length_string() { assert_eq!(verb_length(&[s("hello")], &ctx()).unwrap(), i(5)); }
    #[test] fn length_empty() { assert_eq!(verb_length(&[s("")], &ctx()).unwrap(), i(0)); }
    #[test] fn length_array() { assert_eq!(verb_length(&[arr(vec![i(1), i(2), i(3)])], &ctx()).unwrap(), i(3)); }
    #[test] fn length_null() { assert_eq!(verb_length(&[null()], &ctx()).unwrap(), i(0)); }

    // --- coalesce ---
    #[test] fn coalesce_first_non_null() { assert_eq!(verb_coalesce(&[null(), s(""), s("val")], &ctx()).unwrap(), s("val")); }
    #[test] fn coalesce_first_is_good() { assert_eq!(verb_coalesce(&[s("val"), null()], &ctx()).unwrap(), s("val")); }
    #[test] fn coalesce_all_null() { assert_eq!(verb_coalesce(&[null(), null()], &ctx()).unwrap(), null()); }
    #[test] fn coalesce_all_empty() { assert_eq!(verb_coalesce(&[s(""), s("")], &ctx()).unwrap(), null()); }
    #[test] fn coalesce_int_first() { assert_eq!(verb_coalesce(&[i(0), s("fallback")], &ctx()).unwrap(), i(0)); }

    // =========================================================================
    // Math verbs
    // =========================================================================

    // --- add ---
    #[test] fn add_ints() { assert_eq!(verb_add(&[i(3), i(4)], &ctx()).unwrap(), i(7)); }
    #[test] fn add_floats() { assert_eq!(verb_add(&[f(1.5), f(2.5)], &ctx()).unwrap(), f(4.0)); }
    #[test] fn add_mixed() {
        let result = verb_add(&[i(3), f(1.5)], &ctx()).unwrap();
        assert_eq!(result, f(4.5));
    }
    #[test] fn add_string_coerce() { assert_eq!(verb_add(&[s("3"), s("4")], &ctx()).unwrap(), i(7)); }
    #[test] fn add_too_few() { assert!(verb_add(&[i(1)], &ctx()).is_err()); }
    #[test] fn add_negative() { assert_eq!(verb_add(&[i(-3), i(-4)], &ctx()).unwrap(), i(-7)); }

    // --- subtract ---
    #[test] fn subtract_ints() { assert_eq!(verb_subtract(&[i(10), i(3)], &ctx()).unwrap(), i(7)); }
    #[test] fn subtract_floats() { assert_eq!(verb_subtract(&[f(5.5), f(2.5)], &ctx()).unwrap(), f(3.0)); }
    #[test] fn subtract_negative() { assert_eq!(verb_subtract(&[i(3), i(10)], &ctx()).unwrap(), i(-7)); }
    #[test] fn subtract_too_few() { assert!(verb_subtract(&[i(1)], &ctx()).is_err()); }

    // --- multiply ---
    #[test] fn multiply_ints() { assert_eq!(verb_multiply(&[i(3), i(4)], &ctx()).unwrap(), i(12)); }
    #[test] fn multiply_floats() { assert_eq!(verb_multiply(&[f(2.5), f(4.0)], &ctx()).unwrap(), f(10.0)); }
    #[test] fn multiply_by_zero() { assert_eq!(verb_multiply(&[i(100), i(0)], &ctx()).unwrap(), i(0)); }
    #[test] fn multiply_negative() { assert_eq!(verb_multiply(&[i(-3), i(4)], &ctx()).unwrap(), i(-12)); }
    #[test] fn multiply_too_few() { assert!(verb_multiply(&[i(1)], &ctx()).is_err()); }

    // --- divide ---
    #[test] fn divide_basic() { assert_eq!(verb_divide(&[i(10), i(2)], &ctx()).unwrap(), f(5.0)); }
    #[test] fn divide_float() { assert_eq!(verb_divide(&[f(7.0), f(2.0)], &ctx()).unwrap(), f(3.5)); }
    #[test] fn divide_by_zero() { assert_eq!(verb_divide(&[i(10), i(0)], &ctx()).unwrap(), null()); }
    #[test] fn divide_too_few() { assert!(verb_divide(&[i(1)], &ctx()).is_err()); }

    // --- abs ---
    #[test] fn abs_positive_int() { assert_eq!(verb_abs(&[i(5)], &ctx()).unwrap(), i(5)); }
    #[test] fn abs_negative_int() { assert_eq!(verb_abs(&[i(-5)], &ctx()).unwrap(), i(5)); }
    #[test] fn abs_zero() { assert_eq!(verb_abs(&[i(0)], &ctx()).unwrap(), i(0)); }
    #[test] fn abs_negative_float() { assert_eq!(verb_abs(&[f(-3.14)], &ctx()).unwrap(), f(3.14)); }
    #[test] fn abs_string_num() { assert_eq!(verb_abs(&[s("-42")], &ctx()).unwrap(), i(42)); }

    // --- round ---
    #[test] fn round_basic() { assert_eq!(verb_round(&[f(3.6)], &ctx()).unwrap(), i(4)); }
    #[test] fn round_down() { assert_eq!(verb_round(&[f(3.3)], &ctx()).unwrap(), i(3)); }
    #[test] fn round_int() { assert_eq!(verb_round(&[i(5)], &ctx()).unwrap(), i(5)); }
    #[test] fn round_places() { assert_eq!(verb_round(&[f(3.456), i(2)], &ctx()).unwrap(), f(3.46)); }
    #[test] fn round_half() { assert_eq!(verb_round(&[f(2.5)], &ctx()).unwrap(), i(3)); }

    // --- floor ---
    #[test] fn floor_basic() { assert_eq!(numeric_verbs::floor(&[f(3.7)], &ctx()).unwrap(), i(3)); }
    #[test] fn floor_negative() { assert_eq!(numeric_verbs::floor(&[f(-3.2)], &ctx()).unwrap(), i(-4)); }
    #[test] fn floor_whole() { assert_eq!(numeric_verbs::floor(&[f(5.0)], &ctx()).unwrap(), i(5)); }
    #[test] fn floor_int() { assert_eq!(numeric_verbs::floor(&[i(5)], &ctx()).unwrap(), i(5)); }

    // --- ceil ---
    #[test] fn ceil_basic() { assert_eq!(numeric_verbs::ceil(&[f(3.2)], &ctx()).unwrap(), i(4)); }
    #[test] fn ceil_negative() { assert_eq!(numeric_verbs::ceil(&[f(-3.7)], &ctx()).unwrap(), i(-3)); }
    #[test] fn ceil_whole() { assert_eq!(numeric_verbs::ceil(&[f(5.0)], &ctx()).unwrap(), i(5)); }
    #[test] fn ceil_int() { assert_eq!(numeric_verbs::ceil(&[i(5)], &ctx()).unwrap(), i(5)); }

    // --- mod ---
    #[test] fn mod_basic() { assert_eq!(numeric_verbs::mod_verb(&[i(10), i(3)], &ctx()).unwrap(), i(1)); }
    #[test] fn mod_float() {
        let result = numeric_verbs::mod_verb(&[f(10.5), f(3.0)], &ctx()).unwrap();
        if let DynValue::Float(v) = result { assert!((v - 1.5).abs() < 1e-10); } else { panic!("expected float"); }
    }
    #[test] fn mod_by_zero() { assert_eq!(numeric_verbs::mod_verb(&[i(10), i(0)], &ctx()).unwrap(), null()); }
    #[test] fn mod_too_few() { assert!(numeric_verbs::mod_verb(&[i(10)], &ctx()).is_err()); }

    // --- negate ---
    #[test] fn negate_positive() { assert_eq!(numeric_verbs::negate(&[i(5)], &ctx()).unwrap(), i(-5)); }
    #[test] fn negate_negative() { assert_eq!(numeric_verbs::negate(&[i(-5)], &ctx()).unwrap(), i(5)); }
    #[test] fn negate_zero() { assert_eq!(numeric_verbs::negate(&[i(0)], &ctx()).unwrap(), i(0)); }
    #[test] fn negate_float() { assert_eq!(numeric_verbs::negate(&[f(3.14)], &ctx()).unwrap(), f(-3.14)); }
    #[test] fn negate_string_num() { assert_eq!(numeric_verbs::negate(&[s("42")], &ctx()).unwrap(), f(-42.0)); }
    #[test] fn negate_non_numeric() { assert!(numeric_verbs::negate(&[s("abc")], &ctx()).is_err()); }

    // --- safeDivide ---
    #[test] fn safe_divide_basic() { assert_eq!(numeric_verbs::safe_divide(&[i(10), i(2), i(0)], &ctx()).unwrap(), f(5.0)); }
    #[test] fn safe_divide_by_zero() { assert_eq!(numeric_verbs::safe_divide(&[i(10), i(0), i(-1)], &ctx()).unwrap(), i(-1)); }
    #[test] fn safe_divide_too_few() { assert!(numeric_verbs::safe_divide(&[i(10), i(2)], &ctx()).is_err()); }

    // --- clamp ---
    #[test] fn clamp_in_range() { assert_eq!(numeric_verbs::clamp(&[f(5.0), f(1.0), f(10.0)], &ctx()).unwrap(), f(5.0)); }
    #[test] fn clamp_below_min() { assert_eq!(numeric_verbs::clamp(&[f(-5.0), f(0.0), f(10.0)], &ctx()).unwrap(), f(0.0)); }
    #[test] fn clamp_above_max() { assert_eq!(numeric_verbs::clamp(&[f(15.0), f(0.0), f(10.0)], &ctx()).unwrap(), f(10.0)); }
    #[test] fn clamp_at_min() { assert_eq!(numeric_verbs::clamp(&[f(0.0), f(0.0), f(10.0)], &ctx()).unwrap(), f(0.0)); }
    #[test] fn clamp_at_max() { assert_eq!(numeric_verbs::clamp(&[f(10.0), f(0.0), f(10.0)], &ctx()).unwrap(), f(10.0)); }
    #[test] fn clamp_too_few() { assert!(numeric_verbs::clamp(&[f(5.0), f(0.0)], &ctx()).is_err()); }

    // --- sqrt ---
    #[test] fn sqrt_basic() { assert_eq!(numeric_verbs::sqrt(&[f(9.0)], &ctx()).unwrap(), f(3.0)); }
    #[test] fn sqrt_zero() { assert_eq!(numeric_verbs::sqrt(&[f(0.0)], &ctx()).unwrap(), f(0.0)); }
    #[test] fn sqrt_negative() { assert!(numeric_verbs::sqrt(&[f(-1.0)], &ctx()).is_err()); }
    #[test] fn sqrt_one() { assert_eq!(numeric_verbs::sqrt(&[f(1.0)], &ctx()).unwrap(), f(1.0)); }

    // --- pow ---
    #[test] fn pow_basic() { assert_eq!(numeric_verbs::pow_verb(&[f(2.0), f(3.0)], &ctx()).unwrap(), f(8.0)); }
    #[test] fn pow_zero_exp() { assert_eq!(numeric_verbs::pow_verb(&[f(5.0), f(0.0)], &ctx()).unwrap(), f(1.0)); }
    #[test] fn pow_one_exp() { assert_eq!(numeric_verbs::pow_verb(&[f(5.0), f(1.0)], &ctx()).unwrap(), f(5.0)); }
    #[test] fn pow_negative_exp() {
        let result = numeric_verbs::pow_verb(&[f(2.0), f(-1.0)], &ctx()).unwrap();
        if let DynValue::Float(v) = result { assert!((v - 0.5).abs() < 1e-10); } else { panic!("expected float"); }
    }
    #[test] fn pow_too_few() { assert!(numeric_verbs::pow_verb(&[f(2.0)], &ctx()).is_err()); }

    // --- exp ---
    #[test] fn exp_zero() { assert_eq!(numeric_verbs::exp_verb(&[f(0.0)], &ctx()).unwrap(), f(1.0)); }
    #[test] fn exp_one() {
        let result = numeric_verbs::exp_verb(&[f(1.0)], &ctx()).unwrap();
        if let DynValue::Float(v) = result { assert!((v - std::f64::consts::E).abs() < 1e-10); } else { panic!("expected float"); }
    }

    // --- log ---
    #[test] fn log_natural() {
        let result = numeric_verbs::log_verb(&[f(std::f64::consts::E)], &ctx()).unwrap();
        if let DynValue::Float(v) = result { assert!((v - 1.0).abs() < 1e-10); } else { panic!("expected float"); }
    }
    #[test] fn log_base_10() {
        let result = numeric_verbs::log_verb(&[f(100.0), f(10.0)], &ctx()).unwrap();
        if let DynValue::Float(v) = result { assert!((v - 2.0).abs() < 1e-10); } else { panic!("expected float"); }
    }
    #[test] fn log_zero() { assert!(numeric_verbs::log_verb(&[f(0.0)], &ctx()).is_err()); }
    #[test] fn log_negative() { assert!(numeric_verbs::log_verb(&[f(-1.0)], &ctx()).is_err()); }
    #[test] fn log_base_one() { assert!(numeric_verbs::log_verb(&[f(5.0), f(1.0)], &ctx()).is_err()); }

    // --- ln ---
    #[test] fn ln_e() {
        let result = numeric_verbs::ln(&[f(std::f64::consts::E)], &ctx()).unwrap();
        if let DynValue::Float(v) = result { assert!((v - 1.0).abs() < 1e-10); } else { panic!("expected float"); }
    }
    #[test] fn ln_one() { assert_eq!(numeric_verbs::ln(&[f(1.0)], &ctx()).unwrap(), f(0.0)); }
    #[test] fn ln_negative() { assert!(numeric_verbs::ln(&[f(-1.0)], &ctx()).is_err()); }

    // --- log10 ---
    #[test] fn log10_100() {
        let result = numeric_verbs::log10(&[f(100.0)], &ctx()).unwrap();
        if let DynValue::Float(v) = result { assert!((v - 2.0).abs() < 1e-10); } else { panic!("expected float"); }
    }
    #[test] fn log10_one() { assert_eq!(numeric_verbs::log10(&[f(1.0)], &ctx()).unwrap(), f(0.0)); }
    #[test] fn log10_negative() { assert!(numeric_verbs::log10(&[f(-1.0)], &ctx()).is_err()); }

    // --- sign ---
    #[test] fn sign_positive() { assert_eq!(numeric_verbs::sign(&[f(42.0)], &ctx()).unwrap(), i(1)); }
    #[test] fn sign_negative() { assert_eq!(numeric_verbs::sign(&[f(-42.0)], &ctx()).unwrap(), i(-1)); }
    #[test] fn sign_zero() { assert_eq!(numeric_verbs::sign(&[f(0.0)], &ctx()).unwrap(), i(0)); }
    #[test] fn sign_int() { assert_eq!(numeric_verbs::sign(&[i(-5)], &ctx()).unwrap(), i(-1)); }

    // --- trunc ---
    #[test] fn trunc_positive() { assert_eq!(numeric_verbs::trunc(&[f(3.7)], &ctx()).unwrap(), i(3)); }
    #[test] fn trunc_negative() { assert_eq!(numeric_verbs::trunc(&[f(-3.7)], &ctx()).unwrap(), i(-3)); }
    #[test] fn trunc_whole() { assert_eq!(numeric_verbs::trunc(&[f(5.0)], &ctx()).unwrap(), i(5)); }
    #[test] fn trunc_int() { assert_eq!(numeric_verbs::trunc(&[i(5)], &ctx()).unwrap(), i(5)); }

    // --- minOf / maxOf ---
    #[test] fn min_of_basic() { assert_eq!(numeric_verbs::min_of(&[i(3), i(1), i(5)], &ctx()).unwrap(), i(1)); }
    #[test] fn min_of_single() { assert_eq!(numeric_verbs::min_of(&[i(7)], &ctx()).unwrap(), i(7)); }
    #[test] fn min_of_floats() { assert_eq!(numeric_verbs::min_of(&[f(3.5), f(1.5)], &ctx()).unwrap(), f(1.5)); }
    #[test] fn min_of_empty() { assert!(numeric_verbs::min_of(&[], &ctx()).is_err()); }

    #[test] fn max_of_basic() { assert_eq!(numeric_verbs::max_of(&[i(3), i(1), i(5)], &ctx()).unwrap(), i(5)); }
    #[test] fn max_of_single() { assert_eq!(numeric_verbs::max_of(&[i(7)], &ctx()).unwrap(), i(7)); }
    #[test] fn max_of_floats() { assert_eq!(numeric_verbs::max_of(&[f(3.5), f(1.5)], &ctx()).unwrap(), f(3.5)); }
    #[test] fn max_of_empty() { assert!(numeric_verbs::max_of(&[], &ctx()).is_err()); }

    // --- assert ---
    #[test] fn assert_truthy() { assert_eq!(verb_assert(&[b(true)], &ctx()).unwrap(), b(true)); }
    #[test] fn assert_falsy() { assert!(verb_assert(&[b(false)], &ctx()).is_err()); }
    #[test] fn assert_custom_msg() {
        let err = verb_assert(&[b(false), s("custom error")], &ctx()).unwrap_err();
        assert!(err.contains("custom error"));
    }
    #[test] fn assert_empty() { assert!(verb_assert(&[], &ctx()).is_err()); }

    // =========================================================================
    // Registry tests
    // =========================================================================

    #[test] fn registry_has_core_verbs() {
        let reg = VerbRegistry::new();
        assert!(reg.get("concat").is_some());
        assert!(reg.get("upper").is_some());
        assert!(reg.get("add").is_some());
        assert!(reg.get("eq").is_some());
        assert!(reg.get("isNull").is_some());
    }

    #[test] fn registry_has_extended_verbs() {
        let reg = VerbRegistry::new();
        assert!(reg.get("and").is_some());
        assert!(reg.get("or").is_some());
        assert!(reg.get("xor").is_some());
        assert!(reg.get("between").is_some());
        assert!(reg.get("cond").is_some());
        assert!(reg.get("switch").is_some());
        assert!(reg.get("isFinite").is_some());
        assert!(reg.get("isNaN").is_some());
        assert!(reg.get("safeDivide").is_some());
        assert!(reg.get("clamp").is_some());
        assert!(reg.get("sqrt").is_some());
        assert!(reg.get("pow").is_some());
        assert!(reg.get("exp").is_some());
        assert!(reg.get("log").is_some());
        assert!(reg.get("ln").is_some());
        assert!(reg.get("log10").is_some());
        assert!(reg.get("mod").is_some());
        assert!(reg.get("negate").is_some());
        assert!(reg.get("sign").is_some());
        assert!(reg.get("trunc").is_some());
    }

    #[test] fn registry_custom_verb() {
        let mut reg = VerbRegistry::new();
        fn custom(_args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
            Ok(DynValue::String("custom".to_string()))
        }
        reg.register_custom("myVerb".to_string(), custom);
        assert!(reg.get("myVerb").is_some());
    }

    #[test] fn registry_custom_overrides_builtin() {
        let mut reg = VerbRegistry::new();
        fn custom(_args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
            Ok(DynValue::String("overridden".to_string()))
        }
        reg.register_custom("concat".to_string(), custom);
        let verb = reg.get("concat").unwrap();
        let result = verb(&[], &ctx()).unwrap();
        assert_eq!(result, DynValue::String("overridden".to_string()));
    }

    #[test] fn registry_unknown_verb() {
        let reg = VerbRegistry::new();
        assert!(reg.get("nonExistentVerb").is_none());
    }

    // =========================================================================
    // Truthy helper tests (via ifElse)
    // =========================================================================

    #[test] fn truthy_nonempty_string() { assert_eq!(verb_if_else(&[s("hi"), s("y"), s("n")], &ctx()).unwrap(), s("y")); }
    #[test] fn truthy_empty_string() { assert_eq!(verb_if_else(&[s(""), s("y"), s("n")], &ctx()).unwrap(), s("n")); }
    #[test] fn truthy_nonzero_float() { assert_eq!(verb_if_else(&[f(0.1), s("y"), s("n")], &ctx()).unwrap(), s("y")); }
    #[test] fn truthy_zero_float() { assert_eq!(verb_if_else(&[f(0.0), s("y"), s("n")], &ctx()).unwrap(), s("n")); }
    #[test] fn truthy_array() { assert_eq!(verb_if_else(&[arr(vec![]), s("y"), s("n")], &ctx()).unwrap(), s("y")); }
    #[test] fn truthy_object() { assert_eq!(verb_if_else(&[obj(vec![]), s("y"), s("n")], &ctx()).unwrap(), s("y")); }

    // =========================================================================
    // Cross-type equality edge cases
    // =========================================================================

    #[test] fn eq_string_float_coercion() { assert_eq!(verb_eq(&[s("3.14"), f(3.14)], &ctx()).unwrap(), b(true)); }
    #[test] fn eq_float_string_coercion() { assert_eq!(verb_eq(&[f(3.14), s("3.14")], &ctx()).unwrap(), b(true)); }
    #[test] fn eq_bool_same() { assert_eq!(verb_eq(&[b(false), b(false)], &ctx()).unwrap(), b(true)); }
    #[test] fn eq_bool_diff() { assert_eq!(verb_eq(&[b(true), b(false)], &ctx()).unwrap(), b(false)); }
}

#[cfg(test)]
mod extended_tests_2 {
    use super::*;
    use std::collections::HashMap;

    fn ctx() -> VerbContext {
        VerbContext {
            source: DynValue::Null,
            loop_vars: HashMap::new(),
            accumulators: HashMap::new(),
            tables: HashMap::new(),
        }
    }

    fn s(v: &str) -> DynValue { DynValue::String(v.to_string()) }
    fn i(v: i64) -> DynValue { DynValue::Integer(v) }
    fn f(v: f64) -> DynValue { DynValue::Float(v) }
    fn b(v: bool) -> DynValue { DynValue::Bool(v) }
    fn null() -> DynValue { DynValue::Null }
    fn arr(items: Vec<DynValue>) -> DynValue { DynValue::Array(items) }
    fn obj(pairs: Vec<(&str, DynValue)>) -> DynValue {
        let entries: Vec<(String, DynValue)> = pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect();
        DynValue::Object(entries)
    }

    // =========================================================================
    // Logic verbs — additional edge cases
    // =========================================================================

    // and: non-boolean types should error
    #[test] fn and_string_args() { assert!(verb_and(&[s("true"), s("true")], &ctx()).is_err()); }
    #[test] fn and_null_args() { assert!(verb_and(&[null(), b(true)], &ctx()).is_err()); }
    #[test] fn and_empty_args() { assert!(verb_and(&[], &ctx()).is_err()); }

    // or: non-boolean types should error
    #[test] fn or_non_bool() { assert!(verb_or(&[i(1), i(0)], &ctx()).is_err()); }
    #[test] fn or_null_args() { assert!(verb_or(&[null(), null()], &ctx()).is_err()); }
    #[test] fn or_empty_args() { assert!(verb_or(&[], &ctx()).is_err()); }

    // xor: additional
    #[test] fn xor_null_args() { assert!(verb_xor(&[null(), b(true)], &ctx()).is_err()); }
    #[test] fn xor_empty_args() { assert!(verb_xor(&[], &ctx()).is_err()); }

    // not: object returns false (truthy)
    #[test] fn not_object() { assert_eq!(verb_not(&[obj(vec![])], &ctx()).unwrap(), b(false)); }
    #[test] fn not_nonzero_float() { assert_eq!(verb_not(&[f(1.5)], &ctx()).unwrap(), b(false)); }

    // eq: array and object equality
    #[test] fn eq_arrays_equal() {
        assert_eq!(verb_eq(&[arr(vec![i(1), i(2)]), arr(vec![i(1), i(2)])], &ctx()).unwrap(), b(true));
    }
    #[test] fn eq_arrays_not_equal() {
        assert_eq!(verb_eq(&[arr(vec![i(1)]), arr(vec![i(2)])], &ctx()).unwrap(), b(false));
    }
    #[test] fn eq_null_null() { assert_eq!(verb_eq(&[null(), null()], &ctx()).unwrap(), b(true)); }
    #[test] fn eq_int_string_no_match() { assert_eq!(verb_eq(&[i(42), s("abc")], &ctx()).unwrap(), b(false)); }

    // ne: additional cases
    #[test] fn ne_strings_equal() { assert_eq!(verb_ne(&[s("a"), s("a")], &ctx()).unwrap(), b(false)); }
    #[test] fn ne_strings_differ() { assert_eq!(verb_ne(&[s("a"), s("b")], &ctx()).unwrap(), b(true)); }
    #[test] fn ne_null_null() { assert_eq!(verb_ne(&[null(), null()], &ctx()).unwrap(), b(false)); }
    #[test] fn ne_null_string() { assert_eq!(verb_ne(&[null(), s("x")], &ctx()).unwrap(), b(true)); }

    // lt: mixed int/float
    #[test] fn lt_int_float() { assert_eq!(verb_lt(&[i(3), f(3.5)], &ctx()).unwrap(), b(true)); }
    #[test] fn lt_float_int() { assert_eq!(verb_lt(&[f(3.5), i(3)], &ctx()).unwrap(), b(false)); }
    #[test] fn lt_negative() { assert_eq!(verb_lt(&[i(-5), i(-3)], &ctx()).unwrap(), b(true)); }
    #[test] fn lt_non_comparable() { assert!(verb_lt(&[b(true), b(false)], &ctx()).is_err()); }

    // lte: floats
    #[test] fn lte_floats_equal() { assert_eq!(verb_lte(&[f(3.14), f(3.14)], &ctx()).unwrap(), b(true)); }
    #[test] fn lte_floats_less() { assert_eq!(verb_lte(&[f(1.0), f(2.0)], &ctx()).unwrap(), b(true)); }
    #[test] fn lte_too_few() { assert!(verb_lte(&[i(1)], &ctx()).is_err()); }

    // gt: floats
    #[test] fn gt_floats() { assert_eq!(verb_gt(&[f(5.5), f(3.3)], &ctx()).unwrap(), b(true)); }
    #[test] fn gt_negative_ints() { assert_eq!(verb_gt(&[i(-1), i(-5)], &ctx()).unwrap(), b(true)); }
    #[test] fn gt_too_few() { assert!(verb_gt(&[i(1)], &ctx()).is_err()); }

    // gte: additional
    #[test] fn gte_floats() { assert_eq!(verb_gte(&[f(5.0), f(5.0)], &ctx()).unwrap(), b(true)); }
    #[test] fn gte_strings() { assert_eq!(verb_gte(&[s("b"), s("a")], &ctx()).unwrap(), b(true)); }
    #[test] fn gte_strings_equal() { assert_eq!(verb_gte(&[s("a"), s("a")], &ctx()).unwrap(), b(true)); }
    #[test] fn gte_too_few() { assert!(verb_gte(&[i(1)], &ctx()).is_err()); }

    // between: negative range
    #[test] fn between_negative_range() { assert_eq!(verb_between(&[i(-5), i(-10), i(0)], &ctx()).unwrap(), b(true)); }
    #[test] fn between_float_outside() { assert_eq!(verb_between(&[f(0.5), f(1.0), f(10.0)], &ctx()).unwrap(), b(false)); }
    #[test] fn between_non_comparable() { assert!(verb_between(&[b(true), b(false), b(true)], &ctx()).is_err()); }

    // cond: multi-pair
    #[test] fn cond_multiple_pairs_third_true() {
        assert_eq!(verb_cond(&[b(false), s("a"), b(false), s("b"), b(true), s("c")], &ctx()).unwrap(), s("c"));
    }
    #[test] fn cond_truthy_integer() {
        assert_eq!(verb_cond(&[i(1), s("yes"), s("no")], &ctx()).unwrap(), s("yes"));
    }
    #[test] fn cond_all_false_with_default() {
        assert_eq!(verb_cond(&[b(false), s("a"), b(false), s("b"), s("default")], &ctx()).unwrap(), s("default"));
    }

    // ifElse: truthy string
    #[test] fn if_else_truthy_string() { assert_eq!(verb_if_else(&[s("x"), s("yes"), s("no")], &ctx()).unwrap(), s("yes")); }
    #[test] fn if_else_falsy_empty_str() { assert_eq!(verb_if_else(&[s(""), s("yes"), s("no")], &ctx()).unwrap(), s("no")); }

    // ifNull: non-null int
    #[test] fn if_null_int_value() { assert_eq!(verb_if_null(&[i(0), s("default")], &ctx()).unwrap(), i(0)); }
    #[test] fn if_null_empty_string_not_null() { assert_eq!(verb_if_null(&[s(""), s("default")], &ctx()).unwrap(), s("")); }

    // ifEmpty: non-string types are not "empty"
    #[test] fn if_empty_bool_not_empty() { assert_eq!(verb_if_empty(&[b(false), s("default")], &ctx()).unwrap(), b(false)); }
    #[test] fn if_empty_float_not_empty() { assert_eq!(verb_if_empty(&[f(0.0), s("default")], &ctx()).unwrap(), f(0.0)); }

    // switch: integer values
    #[test] fn switch_int_match() {
        assert_eq!(numeric_verbs::switch_verb(&[i(2), i(1), s("one"), i(2), s("two")], &ctx()).unwrap(), s("two"));
    }
    #[test] fn switch_int_default() {
        assert_eq!(numeric_verbs::switch_verb(&[i(3), i(1), s("one"), s("other")], &ctx()).unwrap(), s("other"));
    }
    #[test] fn switch_single_value_no_pairs() {
        assert_eq!(numeric_verbs::switch_verb(&[s("x")], &ctx()).unwrap(), null());
    }

    // =========================================================================
    // Type checking — additional edge cases
    // =========================================================================

    // isNull: various non-null types
    #[test] fn is_null_bool() { assert_eq!(verb_is_null(&[b(false)], &ctx()).unwrap(), b(false)); }
    #[test] fn is_null_float() { assert_eq!(verb_is_null(&[f(0.0)], &ctx()).unwrap(), b(false)); }
    #[test] fn is_null_array() { assert_eq!(verb_is_null(&[arr(vec![])], &ctx()).unwrap(), b(false)); }
    #[test] fn is_null_object() { assert_eq!(verb_is_null(&[obj(vec![])], &ctx()).unwrap(), b(false)); }

    // isString: edge cases
    #[test] fn is_string_float() { assert_eq!(verb_is_string(&[f(3.14)], &ctx()).unwrap(), b(false)); }
    #[test] fn is_string_bool() { assert_eq!(verb_is_string(&[b(true)], &ctx()).unwrap(), b(false)); }
    #[test] fn is_string_array() { assert_eq!(verb_is_string(&[arr(vec![])], &ctx()).unwrap(), b(false)); }

    // isNumber: edge cases
    #[test] fn is_number_object() { assert_eq!(verb_is_number(&[obj(vec![])], &ctx()).unwrap(), b(false)); }
    #[test] fn is_number_array() { assert_eq!(verb_is_number(&[arr(vec![])], &ctx()).unwrap(), b(false)); }

    // isBoolean: non-bool types
    #[test] fn is_boolean_int() { assert_eq!(verb_is_boolean(&[i(1)], &ctx()).unwrap(), b(false)); }
    #[test] fn is_boolean_null() { assert_eq!(verb_is_boolean(&[null()], &ctx()).unwrap(), b(false)); }
    #[test] fn is_boolean_float() { assert_eq!(verb_is_boolean(&[f(1.0)], &ctx()).unwrap(), b(false)); }

    // isArray: non-array-like string
    #[test] fn is_array_plain_string() { assert_eq!(verb_is_array(&[s("hello")], &ctx()).unwrap(), b(false)); }
    #[test] fn is_array_null() { assert_eq!(verb_is_array(&[null()], &ctx()).unwrap(), b(false)); }

    // isObject: non-object-like string
    #[test] fn is_object_plain_string() { assert_eq!(verb_is_object(&[s("hello")], &ctx()).unwrap(), b(false)); }
    #[test] fn is_object_null() { assert_eq!(verb_is_object(&[null()], &ctx()).unwrap(), b(false)); }
    #[test] fn is_object_int() { assert_eq!(verb_is_object(&[i(42)], &ctx()).unwrap(), b(false)); }
    #[test] fn is_object_empty() { assert_eq!(verb_is_object(&[obj(vec![])], &ctx()).unwrap(), b(true)); }

    // isDate: edge cases
    #[test] fn is_date_short_string() { assert_eq!(verb_is_date(&[s("2024")], &ctx()).unwrap(), b(false)); }
    #[test] fn is_date_empty() { assert_eq!(verb_is_date(&[s("")], &ctx()).unwrap(), b(false)); }
    #[test] fn is_date_bool() { assert_eq!(verb_is_date(&[b(true)], &ctx()).unwrap(), b(false)); }
    #[test] fn is_date_no_args() { assert_eq!(verb_is_date(&[], &ctx()).unwrap(), b(false)); }

    // typeOf: additional types
    #[test] fn type_of_reference() {
        assert_eq!(verb_type_of(&[DynValue::Reference("ref".to_string())], &ctx()).unwrap(), s("reference"));
    }
    #[test] fn type_of_binary() {
        assert_eq!(verb_type_of(&[DynValue::Binary("data".to_string())], &ctx()).unwrap(), s("binary"));
    }
    #[test] fn type_of_date_value() {
        assert_eq!(verb_type_of(&[DynValue::Date("2024-01-01".to_string())], &ctx()).unwrap(), s("date"));
    }

    // isFinite: zero
    #[test] fn is_finite_zero() { assert_eq!(numeric_verbs::is_finite(&[f(0.0)], &ctx()).unwrap(), b(true)); }
    #[test] fn is_finite_negative() { assert_eq!(numeric_verbs::is_finite(&[f(-999.0)], &ctx()).unwrap(), b(true)); }

    // isNaN: infinity is not NaN
    #[test] fn is_nan_infinity() { assert_eq!(numeric_verbs::is_nan(&[f(f64::INFINITY)], &ctx()).unwrap(), b(false)); }
    #[test] fn is_nan_neg_inf() { assert_eq!(numeric_verbs::is_nan(&[f(f64::NEG_INFINITY)], &ctx()).unwrap(), b(false)); }

    // =========================================================================
    // Coercion — additional edge cases
    // =========================================================================

    // coerceString: float formatting
    #[test] fn coerce_str_from_bool_false() { assert_eq!(verb_coerce_string(&[b(false)], &ctx()).unwrap(), s("false")); }
    #[test] fn coerce_str_from_zero_int() { assert_eq!(verb_coerce_string(&[i(0)], &ctx()).unwrap(), s("0")); }
    #[test] fn coerce_str_from_negative() { assert_eq!(verb_coerce_string(&[i(-5)], &ctx()).unwrap(), s("-5")); }

    // coerceNumber: string edge cases
    #[test] fn coerce_num_from_int_string() { assert_eq!(verb_coerce_number(&[s("42")], &ctx()).unwrap(), f(42.0)); }
    #[test] fn coerce_num_from_negative_string() { assert_eq!(verb_coerce_number(&[s("-3.14")], &ctx()).unwrap(), f(-3.14)); }
    #[test] fn coerce_num_from_zero_string() { assert_eq!(verb_coerce_number(&[s("0")], &ctx()).unwrap(), f(0.0)); }

    // coerceBoolean: more string variants
    #[test] fn coerce_bool_n() { assert_eq!(verb_coerce_boolean(&[s("n")], &ctx()).unwrap(), b(false)); }
    #[test] fn coerce_bool_off() { assert_eq!(verb_coerce_boolean(&[s("off")], &ctx()).unwrap(), b(false)); }
    #[test] fn coerce_bool_on() { assert_eq!(verb_coerce_boolean(&[s("on")], &ctx()).unwrap(), b(true)); }
    #[test] fn coerce_bool_1_string() { assert_eq!(verb_coerce_boolean(&[s("1")], &ctx()).unwrap(), b(true)); }
    #[test] fn coerce_bool_from_float_nonzero() { assert_eq!(verb_coerce_boolean(&[f(0.5)], &ctx()).unwrap(), b(true)); }
    #[test] fn coerce_bool_from_float_zero() { assert_eq!(verb_coerce_boolean(&[f(0.0)], &ctx()).unwrap(), b(false)); }

    // coerceInteger: edge cases
    #[test] fn coerce_int_from_negative_float() { assert_eq!(verb_coerce_integer(&[f(-3.9)], &ctx()).unwrap(), i(-3)); }
    #[test] fn coerce_int_from_bool_false() { assert_eq!(verb_coerce_integer(&[b(false)], &ctx()).unwrap(), i(0)); }
    #[test] fn coerce_int_from_large_float() { assert_eq!(verb_coerce_integer(&[f(1e10)], &ctx()).unwrap(), i(10_000_000_000)); }

    // =========================================================================
    // Core string verbs — additional edge cases
    // =========================================================================

    // concat: float and bool
    #[test] fn concat_float() { assert_eq!(verb_concat(&[s("pi="), f(3.14)], &ctx()).unwrap(), s("pi=3.14")); }
    #[test] fn concat_single() { assert_eq!(verb_concat(&[s("only")], &ctx()).unwrap(), s("only")); }
    #[test] fn concat_all_nulls() { assert_eq!(verb_concat(&[null(), null()], &ctx()).unwrap(), s("")); }

    // upper/lower: non-string error
    #[test] fn upper_int_error() { assert!(verb_upper(&[i(42)], &ctx()).is_err()); }
    #[test] fn lower_int_error() { assert!(verb_lower(&[i(42)], &ctx()).is_err()); }
    #[test] fn lower_unicode() { assert_eq!(verb_lower(&[s("CAF\u{00c9}")], &ctx()).unwrap(), s("caf\u{00e9}")); }

    // trim: only whitespace
    #[test] fn trim_all_whitespace() { assert_eq!(verb_trim(&[s("   ")], &ctx()).unwrap(), s("")); }
    #[test] fn trim_int_error() { assert!(verb_trim(&[i(5)], &ctx()).is_err()); }

    // trimLeft/trimRight: edge cases
    #[test] fn trim_left_no_leading() { assert_eq!(verb_trim_left(&[s("hello")], &ctx()).unwrap(), s("hello")); }
    #[test] fn trim_left_int_error() { assert!(verb_trim_left(&[i(5)], &ctx()).is_err()); }
    #[test] fn trim_right_no_trailing() { assert_eq!(verb_trim_right(&[s("hello")], &ctx()).unwrap(), s("hello")); }
    #[test] fn trim_right_int_error() { assert!(verb_trim_right(&[i(5)], &ctx()).is_err()); }

    // capitalize: non-ascii
    #[test] fn capitalize_unicode() { assert_eq!(verb_capitalize(&[s("\u{00e9}cole")], &ctx()).unwrap(), s("\u{00c9}cole")); }
    #[test] fn capitalize_int_error() { assert!(verb_capitalize(&[i(5)], &ctx()).is_err()); }

    // replace: empty search
    #[test] fn replace_empty_search() { assert_eq!(verb_replace(&[s("abc"), s(""), s("x")], &ctx()).unwrap(), s("xaxbxcx")); }
    #[test] fn replace_non_string_error() { assert!(verb_replace(&[i(1), s("a"), s("b")], &ctx()).is_err()); }

    // substring: zero-length
    #[test] fn substring_same_start_end() { assert_eq!(verb_substring(&[s("hello"), i(2), i(2)], &ctx()).unwrap(), s("")); }
    #[test] fn substring_end_beyond() { assert_eq!(verb_substring(&[s("hello"), i(3), i(100)], &ctx()).unwrap(), s("lo")); }

    // length: non-supported types
    #[test] fn length_empty_array() { assert_eq!(verb_length(&[arr(vec![])], &ctx()).unwrap(), i(0)); }
    #[test] fn length_int_error() { assert!(verb_length(&[i(42)], &ctx()).is_err()); }

    // coalesce: mixed
    #[test] fn coalesce_empty_then_int() { assert_eq!(verb_coalesce(&[s(""), i(5)], &ctx()).unwrap(), i(5)); }
    #[test] fn coalesce_null_then_bool() { assert_eq!(verb_coalesce(&[null(), b(true)], &ctx()).unwrap(), b(true)); }
    #[test] fn coalesce_empty() { assert_eq!(verb_coalesce(&[], &ctx()).unwrap(), null()); }

    // =========================================================================
    // Math verbs — additional edge cases
    // =========================================================================

    // add: string coercion
    #[test] fn add_string_numbers() { assert_eq!(verb_add(&[s("3"), s("4")], &ctx()).unwrap(), i(7)); }
    #[test] fn add_negative() { assert_eq!(verb_add(&[i(-3), i(-4)], &ctx()).unwrap(), i(-7)); }
    #[test] fn add_zero() { assert_eq!(verb_add(&[i(0), i(0)], &ctx()).unwrap(), i(0)); }
    #[test] fn add_non_numeric() { assert!(verb_add(&[s("abc"), i(1)], &ctx()).is_err()); }
    #[test] fn add_too_few() { assert!(verb_add(&[i(1)], &ctx()).is_err()); }

    // subtract
    #[test] fn subtract_basic() { assert_eq!(verb_subtract(&[i(10), i(3)], &ctx()).unwrap(), i(7)); }
    #[test] fn subtract_negative_result() { assert_eq!(verb_subtract(&[i(3), i(10)], &ctx()).unwrap(), i(-7)); }
    #[test] fn subtract_floats() { assert_eq!(verb_subtract(&[f(5.5), f(2.5)], &ctx()).unwrap(), f(3.0)); }
    #[test] fn subtract_too_few() { assert!(verb_subtract(&[i(1)], &ctx()).is_err()); }
    #[test] fn subtract_non_numeric() { assert!(verb_subtract(&[s("abc"), i(1)], &ctx()).is_err()); }

    // multiply
    #[test] fn multiply_ints() { assert_eq!(verb_multiply(&[i(3), i(4)], &ctx()).unwrap(), i(12)); }
    #[test] fn multiply_by_zero() { assert_eq!(verb_multiply(&[i(100), i(0)], &ctx()).unwrap(), i(0)); }
    #[test] fn multiply_negative() { assert_eq!(verb_multiply(&[i(-3), i(4)], &ctx()).unwrap(), i(-12)); }
    #[test] fn multiply_floats() { assert_eq!(verb_multiply(&[f(2.5), f(4.0)], &ctx()).unwrap(), f(10.0)); }
    #[test] fn multiply_too_few() { assert!(verb_multiply(&[i(1)], &ctx()).is_err()); }
    #[test] fn multiply_non_numeric() { assert!(verb_multiply(&[s("abc"), i(1)], &ctx()).is_err()); }

    // divide
    #[test] fn divide_basic() { assert_eq!(verb_divide(&[i(10), i(2)], &ctx()).unwrap(), f(5.0)); }
    #[test] fn divide_by_zero() { assert_eq!(verb_divide(&[i(10), i(0)], &ctx()).unwrap(), null()); }
    #[test] fn divide_floats() { assert_eq!(verb_divide(&[f(7.5), f(2.5)], &ctx()).unwrap(), f(3.0)); }
    #[test] fn divide_negative() { assert_eq!(verb_divide(&[i(-10), i(2)], &ctx()).unwrap(), f(-5.0)); }
    #[test] fn divide_too_few() { assert!(verb_divide(&[i(1)], &ctx()).is_err()); }

    // abs
    #[test] fn abs_positive() { assert_eq!(verb_abs(&[i(5)], &ctx()).unwrap(), i(5)); }
    #[test] fn abs_negative() { assert_eq!(verb_abs(&[i(-5)], &ctx()).unwrap(), i(5)); }
    #[test] fn abs_zero() { assert_eq!(verb_abs(&[i(0)], &ctx()).unwrap(), i(0)); }
    #[test] fn abs_negative_float() { assert_eq!(verb_abs(&[f(-3.14)], &ctx()).unwrap(), f(3.14)); }
    #[test] fn abs_string_number() { assert_eq!(verb_abs(&[s("-42")], &ctx()).unwrap(), i(42)); }
    #[test] fn abs_string_float() { assert_eq!(verb_abs(&[s("-3.14")], &ctx()).unwrap(), f(3.14)); }
    #[test] fn abs_string_invalid() { assert!(verb_abs(&[s("abc")], &ctx()).is_err()); }
    #[test] fn abs_null_error() { assert!(verb_abs(&[null()], &ctx()).is_err()); }

    // round
    #[test] fn round_basic() { assert_eq!(verb_round(&[f(3.7)], &ctx()).unwrap(), i(4)); }
    #[test] fn round_down() { assert_eq!(verb_round(&[f(3.2)], &ctx()).unwrap(), i(3)); }
    #[test] fn round_half() { assert_eq!(verb_round(&[f(2.5)], &ctx()).unwrap(), i(3)); }
    #[test] fn round_negative() { assert_eq!(verb_round(&[f(-2.7)], &ctx()).unwrap(), i(-3)); }
    #[test] fn round_int_passthrough() { assert_eq!(verb_round(&[i(42)], &ctx()).unwrap(), i(42)); }
    #[test] fn round_with_places() { assert_eq!(verb_round(&[f(3.14159), i(2)], &ctx()).unwrap(), f(3.14)); }
    #[test] fn round_string_number() { assert_eq!(verb_round(&[s("3.7")], &ctx()).unwrap(), i(4)); }
    #[test] fn round_invalid_string() { assert!(verb_round(&[s("abc")], &ctx()).is_err()); }

    // negate
    #[test] fn negate2_positive_int() { assert_eq!(numeric_verbs::negate(&[i(5)], &ctx()).unwrap(), i(-5)); }
    #[test] fn negate2_negative_int() { assert_eq!(numeric_verbs::negate(&[i(-5)], &ctx()).unwrap(), i(5)); }
    #[test] fn negate2_float() { assert_eq!(numeric_verbs::negate(&[f(3.14)], &ctx()).unwrap(), f(-3.14)); }
    #[test] fn negate2_zero_int() { assert_eq!(numeric_verbs::negate(&[i(0)], &ctx()).unwrap(), i(0)); }
    #[test] fn negate2_string_num() { assert_eq!(numeric_verbs::negate(&[s("7")], &ctx()).unwrap(), f(-7.0)); }
    #[test] fn negate2_non_numeric() { assert!(numeric_verbs::negate(&[s("abc")], &ctx()).is_err()); }
    #[test] fn negate2_null_error() { assert!(numeric_verbs::negate(&[null()], &ctx()).is_err()); }

    // =========================================================================
    // assert verb
    // =========================================================================

    #[test] fn assert_truthy_int() { assert_eq!(verb_assert(&[i(1)], &ctx()).unwrap(), i(1)); }
    #[test] fn assert_falsy_zero() { assert!(verb_assert(&[i(0)], &ctx()).is_err()); }
    #[test] fn assert_truthy_string() { assert_eq!(verb_assert(&[s("yes")], &ctx()).unwrap(), s("yes")); }
    #[test] fn assert_falsy_null() { assert!(verb_assert(&[null()], &ctx()).is_err()); }

    // =========================================================================
    // Registry — comprehensive verb presence
    // =========================================================================

    #[test] fn registry_has_all_logic_verbs() {
        let reg = VerbRegistry::new();
        for name in &["and", "or", "not", "xor", "eq", "ne", "lt", "lte", "gt", "gte", "between", "cond"] {
            assert!(reg.get(name).is_some(), "missing verb: {name}");
        }
    }

    #[test] fn registry_has_all_type_checking_verbs() {
        let reg = VerbRegistry::new();
        for name in &["isNull", "isString", "isNumber", "isBoolean", "isArray", "isObject", "isDate", "typeOf", "isFinite", "isNaN"] {
            assert!(reg.get(name).is_some(), "missing verb: {name}");
        }
    }

    #[test] fn registry_has_all_coercion_verbs() {
        let reg = VerbRegistry::new();
        for name in &["coerceString", "coerceNumber", "coerceBoolean", "coerceInteger", "coerceDate", "coerceTimestamp", "tryCoerce", "toArray", "toObject"] {
            assert!(reg.get(name).is_some(), "missing verb: {name}");
        }
    }

    #[test] fn registry_has_all_core_verbs() {
        let reg = VerbRegistry::new();
        for name in &["concat", "upper", "lower", "trim", "trimLeft", "trimRight", "capitalize", "replace", "substring", "length", "coalesce", "ifNull", "ifEmpty", "ifElse"] {
            assert!(reg.get(name).is_some(), "missing verb: {name}");
        }
    }

    #[test] fn registry_has_all_math_verbs() {
        let reg = VerbRegistry::new();
        for name in &["add", "subtract", "multiply", "divide", "abs", "round", "negate", "switch"] {
            assert!(reg.get(name).is_some(), "missing verb: {name}");
        }
    }
}
