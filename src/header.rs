use rust_htslib::bcf;
use rust_htslib::bcf::header::{HeaderRecord, TagLength, TagType};
use std::cell::Cell;
use std::ffi::CString;
use std::mem::ManuallyDrop;

pub const HEADER_TAG: u16 = 2;
const HEADER_TYPE_NAME: &[u8] = b"Header\0";

#[derive(Debug)]
pub struct Header {
    inner: *mut rust_htslib::htslib::bcf_hdr_t,
    dirty: Cell<bool>,
}

impl Header {
    pub(crate) fn inner_ptr(&self) -> *mut rust_htslib::htslib::bcf_hdr_t {
        self.inner
    }
}

impl Header {
    pub fn new(inner: *mut rust_htslib::htslib::bcf_hdr_t) -> Self {
        Self {
            inner,
            dirty: Cell::new(false),
        }
    }

    fn view(&self) -> ManuallyDrop<bcf::header::HeaderView> {
        // We intentionally suppress Drop here because HeaderView would call
        // `bcf_hdr_destroy()` on the raw pointer it wraps.
        ManuallyDrop::new(bcf::header::HeaderView::new(self.inner))
    }

    pub fn header_records(&self) -> Vec<HeaderRecord> {
        self.view().header_records()
    }

    pub fn info_type(&self, tag: &[u8]) -> Option<(TagType, TagLength)> {
        self.view().info_type(tag).ok()
    }

    pub fn format_type(&self, tag: &[u8]) -> Option<(TagType, TagLength)> {
        self.view().format_type(tag).ok()
    }

    fn sync(&self) {
        if !self.dirty.replace(false) {
            return;
        }
        unsafe {
            rust_htslib::htslib::bcf_hdr_sync(self.inner);
        }
    }

    pub fn push_record(&self, record: &[u8]) -> bool {
        let Ok(c_str) = CString::new(record) else {
            return false;
        };
        let r = unsafe { rust_htslib::htslib::bcf_hdr_append(self.inner, c_str.as_ptr()) };
        self.dirty.set(true);
        self.sync();
        r == 0
    }

    pub fn add_info(&self, id: &str, number: &str, ty: &str, description: &str) -> bool {
        let record = format!(
            "##INFO=<ID={id},Number={number},Type={ty},Description=\"{description}\">"
        );
        self.push_record(record.as_bytes())
    }

    pub fn add_format(&self, id: &str, number: &str, ty: &str, description: &str) -> bool {
        let record = format!(
            "##FORMAT=<ID={id},Number={number},Type={ty},Description=\"{description}\">"
        );
        self.push_record(record.as_bytes())
    }
}


unsafe impl v8::cppgc::GarbageCollected for Header {
    fn trace(&self, _visitor: &mut v8::cppgc::Visitor) {}

    fn get_name(&self) -> &'static std::ffi::CStr {
        unsafe { std::ffi::CStr::from_bytes_with_nul_unchecked(HEADER_TYPE_NAME) }
    }
}

pub fn create_header_object<'a>(
    scope: &mut v8::PinScope<'a, '_>,
    header: Header,
) -> v8::Local<'a, v8::Object> {
    let object_template = v8::ObjectTemplate::new(scope);
    object_template.set_internal_field_count(1);

    let records_key = v8::String::new(scope, "records").unwrap();
    let records_template = v8::FunctionTemplate::new(scope, records_fn);
    object_template.set(records_key.into(), records_template.into());

    let get_key = v8::String::new(scope, "get").unwrap();
    let get_template = v8::FunctionTemplate::new(scope, get_fn);
    object_template.set(get_key.into(), get_template.into());

    let add_info_key = v8::String::new(scope, "addInfo").unwrap();
    let add_info_template = v8::FunctionTemplate::new(scope, add_info_fn);
    object_template.set(add_info_key.into(), add_info_template.into());

    let add_format_key = v8::String::new(scope, "addFormat").unwrap();
    let add_format_template = v8::FunctionTemplate::new(scope, add_format_fn);
    object_template.set(add_format_key.into(), add_format_template.into());

    let object = object_template
        .new_instance(scope)
        .expect("failed to create Header instance");

    let wrapper =
        unsafe { v8::cppgc::make_garbage_collected(scope.get_cpp_heap().unwrap(), header) };
    unsafe {
        v8::Object::wrap::<HEADER_TAG, Header>(scope, object, &wrapper);
    }

    object
}

fn records_fn(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let this = args.this();
    let wrapper = unsafe { v8::Object::unwrap::<HEADER_TAG, Header>(scope, this) }
        .expect("Failed to unwrap Header");
    let header = unsafe { wrapper.as_ref() };

    let records = header.header_records();
    let arr = v8::Array::new(scope, records.len() as i32);
    for (i, record) in records.into_iter().enumerate() {
        let o = v8::Object::new(scope);
        match record {
            HeaderRecord::Info { key, values } => {
                set_str(scope, &o, "type", "INFO");
                set_str(scope, &o, "key", &key);
                set_kv(scope, &o, values);
            }
            HeaderRecord::Format { key, values } => {
                set_str(scope, &o, "type", "FORMAT");
                set_str(scope, &o, "key", &key);
                set_kv(scope, &o, values);
            }
            HeaderRecord::Filter { key, values } => {
                set_str(scope, &o, "type", "FILTER");
                set_str(scope, &o, "key", &key);
                set_kv(scope, &o, values);
            }
            HeaderRecord::Contig { key, values } => {
                set_str(scope, &o, "type", "contig");
                set_str(scope, &o, "key", &key);
                set_kv(scope, &o, values);
            }
            HeaderRecord::Structured { key, values } => {
                set_str(scope, &o, "type", "structured");
                set_str(scope, &o, "key", &key);
                set_kv(scope, &o, values);
            }
            HeaderRecord::Generic { key, value } => {
                set_str(scope, &o, "type", "generic");
                set_str(scope, &o, "key", &key);
                set_str(scope, &o, "value", &value);
            }
        }
        arr.set_index(scope, i as u32, o.into());
    }

    rv.set(arr.into());
}

fn get_fn(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let this = args.this();
    let wrapper = unsafe { v8::Object::unwrap::<HEADER_TAG, Header>(scope, this) }
        .expect("Failed to unwrap Header");
    let header = unsafe { wrapper.as_ref() };

    if args.length() < 2 {
        rv.set(v8::undefined(scope).into());
        return;
    }

    let Ok(section_str) = v8::Local::<v8::String>::try_from(args.get(0)) else {
        rv.set(v8::undefined(scope).into());
        return;
    };
    let Ok(id_str) = v8::Local::<v8::String>::try_from(args.get(1)) else {
        rv.set(v8::undefined(scope).into());
        return;
    };

    let section = section_str.to_rust_string_lossy(scope);
    let id = id_str.to_rust_string_lossy(scope);

    let tag_info = match section.as_str() {
        "INFO" => header.info_type(id.as_bytes()),
        "FORMAT" => header.format_type(id.as_bytes()),
        _ => None,
    };

    let Some((tag_type, tag_length)) = tag_info else {
        rv.set(v8::undefined(scope).into());
        return;
    };

    let o = v8::Object::new(scope);
    set_str(scope, &o, "id", &id);

    let type_str = match tag_type {
        TagType::Flag => "Flag",
        TagType::Integer => "Integer",
        TagType::Float => "Float",
        TagType::String => "String",
    };
    set_str(scope, &o, "type", type_str);

    let number_str = match tag_length {
        TagLength::Fixed(n) => n.to_string(),
        TagLength::AltAlleles => "A".to_string(),
        TagLength::Alleles => "R".to_string(),
        TagLength::Genotypes => "G".to_string(),
        TagLength::Variable => ".".to_string(),
    };
    set_str(scope, &o, "number", &number_str);

    rv.set(o.into());
}

fn add_info_fn(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    add_field_impl(scope, args, &mut rv, |h, id, number, ty, desc| {
        h.add_info(id, number, ty, desc);
    });
}

fn add_format_fn(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    add_field_impl(scope, args, &mut rv, |h, id, number, ty, desc| {
        h.add_format(id, number, ty, desc);
    });
}

fn add_field_impl(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments,
    rv: &mut v8::ReturnValue,
    mut f: impl FnMut(&Header, &str, &str, &str, &str),
) {
    let this = args.this();
    let wrapper = unsafe { v8::Object::unwrap::<HEADER_TAG, Header>(scope, this) }
        .expect("Failed to unwrap Header");
    let header: &Header = unsafe { wrapper.as_ref() };

    if args.length() < 4 {
        rv.set(v8::undefined(scope).into());
        return;
    }

    let Ok(id) = v8::Local::<v8::String>::try_from(args.get(0)) else {
        rv.set(v8::undefined(scope).into());
        return;
    };
    let Ok(number) = v8::Local::<v8::String>::try_from(args.get(1)) else {
        rv.set(v8::undefined(scope).into());
        return;
    };
    let Ok(ty) = v8::Local::<v8::String>::try_from(args.get(2)) else {
        rv.set(v8::undefined(scope).into());
        return;
    };
    let Ok(desc) = v8::Local::<v8::String>::try_from(args.get(3)) else {
        rv.set(v8::undefined(scope).into());
        return;
    };

    let id = id.to_rust_string_lossy(scope);
    let number = number.to_rust_string_lossy(scope);
    let ty = ty.to_rust_string_lossy(scope);
    let desc = desc.to_rust_string_lossy(scope);

    f(header, &id, &number, &ty, &desc);
    rv.set(v8::undefined(scope).into());
}

fn set_str(scope: &mut v8::PinScope<'_, '_>, obj: &v8::Local<v8::Object>, key: &str, value: &str) {
    let k = v8::String::new(scope, key).unwrap();
    let v = v8::String::new(scope, value).unwrap();
    obj.set(scope, k.into(), v.into());
}

fn set_kv(
    scope: &mut v8::PinScope<'_, '_>,
    obj: &v8::Local<v8::Object>,
    values: impl IntoIterator<Item = (String, String)>,
) {
    for (k, v) in values {
        set_str(scope, obj, &k, &v);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_htslib::bcf::Read;
    use std::fs;
    use std::path::PathBuf;

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

    fn eval_header_js(path: &str, js_expr: &str) -> String {
        let platform = crate::runtime::ensure_v8_initialized().clone();
        let _guard = crate::runtime::v8_lock();

        let heap = v8::cppgc::Heap::create(platform, v8::cppgc::HeapCreateParams::default());
        let isolate = &mut v8::Isolate::new(v8::CreateParams::default().cpp_heap(heap));

        v8::scope!(handle_scope, isolate);
        let context = v8::Context::new(handle_scope, Default::default());
        let scope = &mut v8::ContextScope::new(handle_scope, context);

        let reader = bcf::Reader::from_path(path).unwrap();
        let header_obj = create_header_object(scope, Header::new(reader.header().inner));

        let code = v8::String::new(scope, js_expr).unwrap();
        let script = v8::Script::compile(scope, code, None).unwrap();

        let global = context.global(scope);
        let header_name = v8::String::new(scope, "header").unwrap();
        global.set(scope, header_name.into(), header_obj.into());

        let result = script.run(scope).unwrap();
        result.to_string(scope).unwrap().to_rust_string_lossy(scope)
    }

    #[test]
    fn test_header_records_and_get() {
        let path = tmp_path("header_api.vcf");
        let vcf = "##fileformat=VCFv4.2\n\
##INFO=<ID=DP,Number=1,Type=Integer,Description=\"Depth\">\n\
##FORMAT=<ID=GT,Number=1,Type=String,Description=\"Genotype\">\n\
##contig=<ID=chr1>\n\
#CHROM\tPOS\tID\tREF\tALT\tQUAL\tFILTER\tINFO\tFORMAT\tS1\n\
chr1\t1\t.\tA\tC\t.\t.\tDP=7\tGT\t0/1\n";
        fs::write(&path, vcf).unwrap();
        let path = path.to_str().unwrap();

        assert_eq!(eval_header_js(path, "header.get('INFO','DP').type"), "Integer");
        assert_eq!(eval_header_js(path, "header.get('INFO','DP').number"), "1");
        assert_eq!(eval_header_js(path, "header.get('FORMAT','GT').type"), "String");
        assert_eq!(eval_header_js(path, "header.get('FORMAT','GT').number"), "1");

        assert_eq!(
            eval_header_js(path, "header.records().filter(r => r.type === 'INFO').length"),
            "1"
        );
        assert_eq!(
            eval_header_js(path, "header.records().filter(r => r.type === 'FORMAT').length"),
            "1"
        );
        assert_eq!(
            eval_header_js(
                path,
                "(() => { const r = header.records().filter(r => r.type === 'INFO')[0]; return r ? (r.ID + ':' + r.Description.replaceAll('\\\"','')) : 'missing'; })()",
            ),
            "DP:Depth"
        );

        let _ = fs::remove_file(path);
    }

    #[test]
    fn test_add_info_and_format_update_header_lookup() {
        let path = tmp_path("add_header_fields.vcf");
        let vcf = "##fileformat=VCFv4.2\n\
##INFO=<ID=DP,Number=1,Type=Integer,Description=\"Depth\">\n\
##contig=<ID=chr1>\n\
#CHROM\tPOS\tID\tREF\tALT\tQUAL\tFILTER\tINFO\n\
chr1\t1\t.\tA\tC\t.\t.\tDP=7\n";
        fs::write(&path, vcf).unwrap();
        let path = path.to_str().unwrap();

        assert_eq!(eval_header_js(path, "header.get('INFO','ZZ')"), "undefined");
        assert_eq!(
            eval_header_js(
                path,
                "header.addInfo('ZZ','1','Integer','Zed'); header.get('INFO','ZZ').type",
            ),
            "Integer"
        );
        assert_eq!(
            eval_header_js(
                path,
                "header.addInfo('ZZ','1','Integer','Zed'); header.get('INFO','ZZ').number",
            ),
            "1"
        );

        assert_eq!(eval_header_js(path, "header.get('FORMAT','ZZ')"), "undefined");
        assert_eq!(
            eval_header_js(
                path,
                "header.addFormat('ZZ','1','Integer','Zed'); header.get('FORMAT','ZZ').type",
            ),
            "Integer"
        );

        let _ = fs::remove_file(path);
    }
}
