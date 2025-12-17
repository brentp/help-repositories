use rust_htslib::bcf::{self, Read};
use v8;

mod variant;

use variant::{create_object_template, create_variant_object, Variant};

type AnyError = Box<dyn std::error::Error>;

const V8_FLAGS: &str = "--no_freeze_flags_after_init --expose-gc";
const GC_EVERY: usize = 100_000;

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

