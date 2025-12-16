use rust_htslib::bcf::{self, Read};
use v8;

mod variant;

use variant::{InfoField, InfoValue, Variant};

type AnyError = Box<dyn std::error::Error>;

const TAG: u16 = 1;
const V8_FLAGS: &str = "--no_freeze_flags_after_init --expose-gc";
const GC_EVERY: usize = 100_000;

fn attr_getter(
    scope: &mut v8::PinScope<'_, '_>,
    key: v8::Local<v8::Name>,
    args: v8::PropertyCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let this = args.this();
    let wrapper = unsafe { v8::Object::unwrap::<TAG, Variant>(scope, this) }
        .expect("Failed to unwrap Variant");
    let variant = unsafe { wrapper.as_ref() };

    match key.to_rust_string_lossy(scope).as_bytes() {
        b"start" => {
            rv.set(v8::Number::new(scope, variant.start() as f64).into());
        }
        b"pos" => {
            rv.set(v8::Number::new(scope, variant.pos() as f64).into());
        }
        b"stop" => {
            rv.set(v8::Number::new(scope, variant.end() as f64).into());
        }
        b"chrom" => {
            let name_str = v8::String::new(scope, variant.chrom()).unwrap();
            rv.set(name_str.into());
        }
        b"id" => {
            let s = v8::String::new(scope, &variant.id()).unwrap();
            rv.set(s.into());
        }
        b"ref" => {
            let s = v8::String::new(scope, &variant.reference()).unwrap();
            rv.set(s.into());
        }
        b"alt" => {
            let values = variant
                .alts()
                .into_iter()
                .map(|s| v8::String::new(scope, &s).unwrap().into())
                .collect::<Vec<v8::Local<v8::Value>>>();
            let arr = v8::Array::new_with_elements(scope, &values);
            rv.set(arr.into());
        }
        b"qual" => match variant.qual() {
            Some(q) => rv.set(v8::Number::new(scope, q as f64).into()),
            None => rv.set(v8::null(scope).into()),
        },
        _ => {
            let message = v8::String::new(scope, "Invalid key").unwrap();
            let error = v8::Exception::error(scope, message);
            rv.set(error.into());
        }
    }
}

fn info_fn(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let this = args.this();
    let wrapper = unsafe { v8::Object::unwrap::<TAG, Variant>(scope, this) }
        .expect("Failed to unwrap Variant");
    let variant = unsafe { wrapper.as_ref() };

    if args.length() < 1 {
        rv.set(v8::undefined(scope).into());
        return;
    }

    let tag = args.get(0);
    let Ok(tag_str) = v8::Local::<v8::String>::try_from(tag) else {
        rv.set(v8::undefined(scope).into());
        return;
    };
    let tag = tag_str.to_rust_string_lossy(scope);

    let value = match variant.info(tag.as_bytes()) {
        Ok(v) => v,
        Err(_) => {
            rv.set(v8::undefined(scope).into());
            return;
        }
    };

    match value {
        None => rv.set(v8::undefined(scope).into()),
        Some(InfoValue::Flag(b)) => rv.set(v8::Boolean::new(scope, b).into()),
        Some(InfoValue::Integer(field)) => rv.set(field_to_value(scope, field, |scope, v| {
            v8::Number::new(scope, v as f64).into()
        })),
        Some(InfoValue::Float(field)) => rv.set(field_to_value(scope, field, |scope, v| {
            v8::Number::new(scope, v as f64).into()
        })),
        Some(InfoValue::String(field)) => rv.set(field_to_value(scope, field, |scope, v| {
            v8::String::new(scope, &v).unwrap().into()
        })),
    }
}

fn field_to_value<'s, 'i, T>(
    scope: &mut v8::PinScope<'s, 'i>,
    field: InfoField<T>,
    mut to_value: impl FnMut(&mut v8::PinScope<'s, 'i>, T) -> v8::Local<'s, v8::Value>,
) -> v8::Local<'s, v8::Value> {
    match field {
        InfoField::Scalar(v) => match v {
            Some(v) => to_value(scope, v),
            None => v8::null(scope).into(),
        },
        InfoField::Array(values) => {
            let arr = v8::Array::new(scope, values.len() as i32);
            for (i, v) in values.into_iter().enumerate() {
                let v = match v {
                    Some(v) => to_value(scope, v),
                    None => v8::null(scope).into(),
                };
                arr.set_index(scope, i as u32, v);
            }
            arr.into()
        }
    }
}

fn create_object_template<'a>(
    scope: &mut v8::PinScope<'a, '_>,
) -> v8::Local<'a, v8::ObjectTemplate> {
    let object_template = v8::ObjectTemplate::new(scope);
    object_template.set_internal_field_count(1);

    for key in ["start", "pos", "stop", "chrom", "id", "ref", "alt", "qual"] {
        let name = v8::String::new(scope, key).unwrap();
        object_template.set_accessor(name.into(), attr_getter);
    }

    let info_key = v8::String::new(scope, "info").unwrap();
    let info_template = v8::FunctionTemplate::new(scope, info_fn);
    object_template.set(info_key.into(), info_template.into());

    object_template
}

fn create_variant_object<'a>(
    scope: &mut v8::PinScope<'a, '_>,
    object_template: v8::Local<'a, v8::ObjectTemplate>,
    variant: Variant,
) -> v8::Local<'a, v8::Object> {
    let object = object_template
        .new_instance(scope)
        .expect("failed to create Variant instance");

    let wrapper =
        unsafe { v8::cppgc::make_garbage_collected(scope.get_cpp_heap().unwrap(), variant) };
    unsafe {
        v8::Object::wrap::<TAG, Variant>(scope, object, &wrapper);
    }

    object
}

fn maybe_force_gc(scope: &mut v8::PinScope<'_, '_>) {
    scope.request_garbage_collection_for_testing(v8::GarbageCollectionType::Full);
    unsafe {
        scope.get_cpp_heap()
            .unwrap()
            .collect_garbage_for_testing(v8::cppgc::EmbedderStackState::MayContainHeapPointers);
    }
}

fn run(path: &str, js_expr: &str) -> Result<(), AnyError> {
    // Initialize V8 with cppgc
    let platform = v8::new_unprotected_default_platform(0, false).make_shared();
    v8::V8::set_flags_from_string(V8_FLAGS);

    v8::V8::initialize_platform(platform.clone());
    v8::cppgc::initialize_process(platform.clone());
    v8::V8::initialize();

    {
        let heap = v8::cppgc::Heap::create(platform, v8::cppgc::HeapCreateParams::default());
        let isolate = &mut v8::Isolate::new(v8::CreateParams::default().cpp_heap(heap));

        v8::scope!(handle_scope, isolate);
        let context = v8::Context::new(handle_scope, Default::default());
        let scope = &mut v8::ContextScope::new(handle_scope, context);

        let object_template_local = create_object_template(scope);
        let object_template = v8::Global::new(scope, object_template_local);

        let mut reader = bcf::Reader::from_path(path)?;

        let code = v8::String::new(scope, js_expr).unwrap();
        let script = v8::Script::compile(scope, code, None).unwrap();
        let global = context.global(scope);
        let variant_name = v8::String::new(scope, "variant").unwrap();

        for (i, result) in reader.records().enumerate() {
            let record = result?;
            let record = Variant::from_record(record);

            // Fresh scope each loop so Local handles drop promptly.
            v8::scope!(loop_scope, scope);
            let object_template = v8::Local::new(loop_scope, &object_template);

            let variant_object = create_variant_object(loop_scope, object_template, record);
            global.set(loop_scope, variant_name.into(), variant_object.into());

            let result = script.run(loop_scope).unwrap();
            let result_str = result.to_string(loop_scope).unwrap();
            println!("{}", result_str.to_rust_string_lossy(loop_scope));

            // Periodically force GC to observe steady-state behavior.
            if i != 0 && i % GC_EVERY == 0 {
                maybe_force_gc(loop_scope);
            }
        }
    }

    // cleanup
    unsafe {
        v8::cppgc::shutdown_process();
        v8::V8::dispose();
    }
    v8::V8::dispose_platform();

    eprintln!("done");
    Ok(())
}

fn main() -> Result<(), AnyError> {
    let mut args = std::env::args();
    let program = args.next().unwrap_or_else(|| "v8_hts".to_string());
    let Some(path) = args.next() else {
        eprintln!("usage: {program} <input.vcf|input.bcf> [js_expr]");
        eprintln!("example: {program} input.vcf.gz 'variant.chrom + \":\" + variant.start'");
        return Ok(());
    };
    let js_expr = args.next().unwrap_or_else(|| "variant.start".to_string());
    run(&path, &js_expr)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, OnceLock};

    static V8_LOCK: Mutex<()> = Mutex::new(());
    static PLATFORM: OnceLock<v8::SharedRef<v8::Platform>> = OnceLock::new();

    fn ensure_v8_initialized() -> &'static v8::SharedRef<v8::Platform> {
        PLATFORM.get_or_init(|| {
            let platform = v8::new_unprotected_default_platform(0, false).make_shared();
            v8::V8::set_flags_from_string(V8_FLAGS);
            v8::V8::initialize_platform(platform.clone());
            v8::cppgc::initialize_process(platform.clone());
            v8::V8::initialize();
            platform
        })
    }

    fn eval_js(path: &str, js_expr: &str) -> String {
        let platform = ensure_v8_initialized().clone();
        let _guard = V8_LOCK.lock().unwrap_or_else(|e| e.into_inner());

        let heap = v8::cppgc::Heap::create(platform, v8::cppgc::HeapCreateParams::default());
        let isolate = &mut v8::Isolate::new(v8::CreateParams::default().cpp_heap(heap));

        v8::scope!(handle_scope, isolate);
        let context = v8::Context::new(handle_scope, Default::default());
        let scope = &mut v8::ContextScope::new(handle_scope, context);

        let object_template_local = create_object_template(scope);
        let object_template = v8::Global::new(scope, object_template_local);

        let mut reader = bcf::Reader::from_path(path).unwrap();
        let record = reader.records().next().unwrap().unwrap();
        let record = Variant::from_record(record);

        let code = v8::String::new(scope, js_expr).unwrap();
        let script = v8::Script::compile(scope, code, None).unwrap();

        let global = context.global(scope);
        let variant_name = v8::String::new(scope, "variant").unwrap();
        let object_template = v8::Local::new(scope, &object_template);
        let variant_object = create_variant_object(scope, object_template, record);
        global.set(scope, variant_name.into(), variant_object.into());

        let result = script.run(scope).unwrap();
        result.to_string(scope).unwrap().to_rust_string_lossy(scope)
    }

    #[test]
    fn test_js_info_scalar_and_array() {
        let path = "tests/t.vcf.gz";
        assert_eq!(eval_js(path, "variant.info('DP')"), "10");
        assert_eq!(eval_js(path, "variant.info('NOPE')"), "undefined");
    }

    #[test]
    fn test_js_variant_attributes() {
        let path = "tests/t.vcf.gz";
        assert_eq!(eval_js(path, "variant.chrom"), "chr1");
        assert_eq!(eval_js(path, "variant.pos"), "1000");
        assert_eq!(eval_js(path, "variant.start"), "999");
        assert_eq!(eval_js(path, "variant.stop"), "1000");
        assert_eq!(eval_js(path, "variant.ref"), "A");
        assert_eq!(eval_js(path, "variant.alt.length"), "1");
        assert_eq!(eval_js(path, "variant.alt[0]"), "C");
        assert_eq!(eval_js(path, "variant.id"), ".");
        assert_eq!(eval_js(path, "variant.qual === null"), "true");
    }
}
