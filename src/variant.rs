use rust_htslib::bcf;
use rust_htslib::bcf::header::{TagLength, TagType};
use rust_htslib::bcf::record::Numeric;
use v8;

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
