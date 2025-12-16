use v8;

#[derive(Debug)]
struct Variant {
    _chrom: String,
    _start: i32,
    _end: i32,
    _make_this_use_memory: [u64; 128],
}

impl Variant {
    fn new(chrom: String, start: i32, end: i32) -> Self {
        Self {
            _chrom: chrom,
            _start: start,
            _end: end,
            _make_this_use_memory: [0; 128],
        }
    }

    fn start(&self) -> i64 {
        self._start as i64
    }
    fn end(&self) -> i64 {
        self._end as i64
    }
    fn chrom(&self) -> &str {
        &self._chrom
    }
}

/*
impl Drop for Variant {
    fn drop(&mut self) {
    }
}
*/

unsafe impl v8::cppgc::GarbageCollected for Variant {
    fn trace(&self, _visitor: &mut v8::cppgc::Visitor) {}

    fn get_name(&self) -> &'static std::ffi::CStr {
        c"Variant"
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
        .expect("Failed to unwrap VariantWrapper");
    let variant = unsafe { wrapper.as_ref() };

    match key.to_rust_string_lossy(scope).as_bytes() {
        b"start" => {
            rv.set(v8::Number::new(scope, variant.start() as f64).into());
        }
        b"stop" => {
            rv.set(v8::Number::new(scope, variant.end() as f64).into());
        }
        b"chrom" => {
            let name_str = v8::String::new(scope, variant.chrom()).unwrap();
            rv.set(name_str.into());
        }
        _ => {
            let message = v8::String::new(scope, "Invalid key").unwrap();
            let error = v8::Exception::error(scope, message);
            rv.set(error.into());
        }
    }
}

const TAG: u16 = 1;

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

    let wrapper = unsafe { v8::cppgc::make_garbage_collected(scope.get_cpp_heap().unwrap(), variant) };

    unsafe {
        v8::Object::wrap::<TAG, Variant>(scope, object, &wrapper);
    }

    object
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize V8 with cppgc
    let platform = v8::new_unprotected_default_platform(0, false).make_shared();

    v8::V8::set_flags_from_string("--no_freeze_flags_after_init --expose-gc");

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

        let code = v8::String::new(scope, "variant.start").unwrap();
        let script = v8::Script::compile(scope, code, None).unwrap();
        let global = context.global(scope);
        let variant_name = v8::String::new(scope, "variant").unwrap();

        fn current_rss_kb() -> Option<usize> {
            let status = std::fs::read_to_string("/proc/self/status").ok()?;
            for line in status.lines() {
                if let Some(rest) = line.strip_prefix("VmRSS:") {
                    return rest.trim().split_whitespace().next()?.parse().ok();
                }
            }
            None
        }

        let n = 200_000_000;
        for i in 0..n {
            let record = Variant::new("chr1".to_string(), i, i + 1);

            // Create a fresh scope each loop, so Local handles drop.
            v8::scope!(loop_scope, scope);
            let object_template = v8::Local::new(loop_scope, &object_template);

            let variant_object = create_variant_object(loop_scope, object_template, record);
            global.set(loop_scope, variant_name.into(), variant_object.into());

            // Run the JavaScript code
            let result = script.run(loop_scope).unwrap();

            // Convert the result to a string and print it
            /*
            if i % 500_000 == 0 {
                let result_str = result.to_string(loop_scope).unwrap();
                let rss_kb = current_rss_kb().unwrap_or(0);
                println!(
                    "variant.start: {}, /{} (VmRSS {} KB)",
                    result_str.to_rust_string_lossy(loop_scope),
                    n,
                    rss_kb
                );
            }
            */

            // Periodically force GC to observe steady-state behavior.
            if i % 1_000_000 == 0 {
                loop_scope.request_garbage_collection_for_testing(v8::GarbageCollectionType::Full);
                unsafe {
                    loop_scope
                        .get_cpp_heap()
                        .unwrap()
                        .collect_garbage_for_testing(v8::cppgc::EmbedderStackState::MayContainHeapPointers);
                }
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
    std::thread::sleep(std::time::Duration::from_secs(2));

    Ok(())
}
