use rust_htslib::bcf::{self, Read};
use v8;

type AnyError = Box<dyn std::error::Error>;

const TAG: u16 = 1;
const V8_FLAGS: &str = "--no_freeze_flags_after_init --expose-gc";
const GC_EVERY: usize = 100_000;
const VARIANT_TYPE_NAME: &[u8] = b"Variant\0";

#[derive(Debug)]
struct Variant {
    record: bcf::Record,
    chrom: String,
}

impl Variant {
    fn from_record(mut record: bcf::Record) -> Self {
        record.unpack();
        let chrom = match record.rid() {
            Some(rid) => record
                .header()
                .rid2name(rid)
                .ok()
                .map(|name| String::from_utf8_lossy(name).into_owned())
                .unwrap_or_else(|| ".".to_string()),
            None => ".".to_string(),
        };
        Self { record, chrom }
    }

    fn start(&self) -> i64 {
        self.record.pos()
    }
    fn end(&self) -> i64 {
        self.record.end()
    }
    fn chrom(&self) -> &str {
        &self.chrom
    }
}

unsafe impl v8::cppgc::GarbageCollected for Variant {
    fn trace(&self, _visitor: &mut v8::cppgc::Visitor) {}

    fn get_name(&self) -> &'static std::ffi::CStr {
        unsafe { std::ffi::CStr::from_bytes_with_nul_unchecked(VARIANT_TYPE_NAME) }
    }
}

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
        b"stop" => {
            rv.set(v8::Number::new(scope, variant.end() as f64).into());
        }
        b"chrom" => {
            let name = variant.chrom();
            let name_str = v8::String::new(scope, name).unwrap();
            rv.set(name_str.into());
        }
        _ => {
            let message = v8::String::new(scope, "Invalid key").unwrap();
            let error = v8::Exception::error(scope, message);
            rv.set(error.into());
        }
    }
}

fn create_object_template<'a>(
    scope: &mut v8::PinScope<'a, '_>,
) -> v8::Local<'a, v8::ObjectTemplate> {
    let object_template = v8::ObjectTemplate::new(scope);
    object_template.set_internal_field_count(1);

    let start_name = v8::String::new(scope, "start").unwrap();
    let stop_name = v8::String::new(scope, "stop").unwrap();
    let chrom_name = v8::String::new(scope, "chrom").unwrap();
    object_template.set_accessor(start_name.into(), attr_getter);
    object_template.set_accessor(stop_name.into(), attr_getter);
    object_template.set_accessor(chrom_name.into(), attr_getter);

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
