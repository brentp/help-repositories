use rust_htslib::bcf;
use rust_htslib::bcf::header::{TagLength, TagType};
use rust_htslib::bcf::record::Numeric;
use v8;

pub const TAG: u16 = 1;
const VARIANT_TYPE_NAME: &[u8] = b"Variant\0";

#[derive(Debug, Clone, PartialEq)]
pub enum InfoField<T> {
    Scalar(Option<T>),
    Array(Vec<Option<T>>),
}

#[derive(Debug, Clone, PartialEq)]
pub enum InfoValue {
    Flag(bool),
    Integer(InfoField<i32>),
    Float(InfoField<f32>),
    String(InfoField<String>),
}

#[derive(Debug)]
pub struct Variant {
    record: bcf::Record,
    chrom: String,
}

impl Variant {
    pub fn from_record(mut record: bcf::Record) -> Self {
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

    pub fn chrom(&self) -> &str {
        &self.chrom
    }

    pub fn start(&self) -> i64 {
        self.record.pos()
    }

    pub fn pos(&self) -> i64 {
        self.record.pos() + 1
    }

    pub fn end(&self) -> i64 {
        self.record.end()
    }

    pub fn id(&self) -> String {
        String::from_utf8_lossy(&self.record.id()).into_owned()
    }

    pub fn reference(&self) -> String {
        self.record
            .alleles()
            .first()
            .map(|a| String::from_utf8_lossy(a).into_owned())
            .unwrap_or_else(|| ".".to_string())
    }

    pub fn alts(&self) -> Vec<String> {
        self.record
            .alleles()
            .into_iter()
            .skip(1)
            .map(|a| String::from_utf8_lossy(a).into_owned())
            .collect()
    }

    pub fn qual(&self) -> Option<f32> {
        let qual = self.record.qual();
        if qual.is_missing() {
            None
        } else {
            Some(qual)
        }
    }

    pub fn info(&self, tag: &[u8]) -> Result<Option<InfoValue>, Box<dyn std::error::Error>> {
        let (tag_type, tag_length) = self.record.header().info_type(tag)?;

        match tag_type {
            TagType::Flag => Ok(Some(InfoValue::Flag(self.record.info(tag).flag()?))),
            TagType::Integer => {
                let Some(values) = self.record.info(tag).integer()? else {
                    return Ok(None);
                };
                let values = values
                    .iter()
                    .map(|v| if v.is_missing() { None } else { Some(*v) })
                    .collect::<Vec<_>>();
                Ok(Some(InfoValue::Integer(cardinality(values, tag_length))))
            }
            TagType::Float => {
                let Some(values) = self.record.info(tag).float()? else {
                    return Ok(None);
                };
                let values = values
                    .iter()
                    .map(|v| if v.is_missing() { None } else { Some(*v) })
                    .collect::<Vec<_>>();
                Ok(Some(InfoValue::Float(cardinality(values, tag_length))))
            }
            TagType::String => {
                let Some(values) = self.record.info(tag).string()? else {
                    return Ok(None);
                };
                let values = values
                    .iter()
                    .map(|s| Some(String::from_utf8_lossy(s).into_owned()))
                    .collect::<Vec<_>>();
                Ok(Some(InfoValue::String(cardinality(values, tag_length))))
            }
        }
    }
}

fn cardinality<T>(values: Vec<Option<T>>, tag_length: TagLength) -> InfoField<T> {
    match tag_length {
        TagLength::Fixed(1) => InfoField::Scalar(values.into_iter().next().unwrap_or(None)),
        _ => InfoField::Array(values),
    }
}

unsafe impl v8::cppgc::GarbageCollected for Variant {
    fn trace(&self, _visitor: &mut v8::cppgc::Visitor) {}

    fn get_name(&self) -> &'static std::ffi::CStr {
        unsafe { std::ffi::CStr::from_bytes_with_nul_unchecked(VARIANT_TYPE_NAME) }
    }
}

pub fn create_object_template<'a>(
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

pub fn create_variant_object<'a>(
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

#[cfg(test)]
mod tests {
    use super::*;
    use rust_htslib::bcf::Read;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::{Mutex, OnceLock};

    const V8_FLAGS: &str = "--no_freeze_flags_after_init --expose-gc";

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

    fn tmp_path(file_name: &str) -> PathBuf {
        let mut path = std::env::temp_dir();
        path.push(format!(
            "v8_hts_{}_{}_{}",
            std::process::id(),
            file_name,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        path
    }

    #[test]
    fn test_variant_basic_fields() {
        let mut reader = bcf::Reader::from_path("tests/t.vcf.gz").unwrap();
        let record = reader.records().next().unwrap().unwrap();
        let variant = Variant::from_record(record);

        assert_eq!(variant.chrom(), "chr1");
        assert_eq!(variant.pos(), 1000);
        assert_eq!(variant.start(), 999);
        assert_eq!(variant.end(), 1000);
        assert_eq!(variant.id(), ".");
        assert_eq!(variant.reference(), "A");
        assert_eq!(variant.alts(), vec!["C".to_string()]);
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

    #[test]
    fn test_variant_info_uses_header_type_and_number() {
        let path = tmp_path("info.vcf");
        let vcf = "##fileformat=VCFv4.2\n\
##INFO=<ID=DP,Number=1,Type=Integer,Description=\"Depth\">\n\
##INFO=<ID=AF,Number=2,Type=Float,Description=\"Allele frequencies\">\n\
##INFO=<ID=NOTE,Number=1,Type=String,Description=\"Note\">\n\
##INFO=<ID=FLAGS,Number=.,Type=String,Description=\"Flags\">\n\
##INFO=<ID=SOMATIC,Number=0,Type=Flag,Description=\"Somatic\">\n\
##contig=<ID=chr1>\n\
#CHROM\tPOS\tID\tREF\tALT\tQUAL\tFILTER\tINFO\n\
chr1\t1\t.\tA\tC,G\t.\t.\tDP=7;AF=0.1,0.2;NOTE=hi;FLAGS=a,b,c;SOMATIC\n";
        fs::write(&path, vcf).unwrap();

        let mut reader = bcf::Reader::from_path(&path).unwrap();
        let record = reader.records().next().unwrap().unwrap();
        let variant = Variant::from_record(record);

        assert_eq!(
            variant.info(b"DP").unwrap(),
            Some(InfoValue::Integer(InfoField::Scalar(Some(7))))
        );

        match variant.info(b"AF").unwrap() {
            Some(InfoValue::Float(InfoField::Array(values))) => {
                assert_eq!(values.len(), 2);
                assert!((values[0].unwrap() - 0.1).abs() < 1e-6);
                assert!((values[1].unwrap() - 0.2).abs() < 1e-6);
            }
            other => panic!("unexpected AF value: {other:?}"),
        }

        assert_eq!(
            variant.info(b"NOTE").unwrap(),
            Some(InfoValue::String(InfoField::Scalar(Some("hi".to_string()))))
        );

        assert_eq!(
            variant.info(b"FLAGS").unwrap(),
            Some(InfoValue::String(InfoField::Array(vec![
                Some("a".to_string()),
                Some("b".to_string()),
                Some("c".to_string()),
            ])))
        );

        assert_eq!(variant.info(b"SOMATIC").unwrap(), Some(InfoValue::Flag(true)));

        let _ = fs::remove_file(&path);
    }
}
