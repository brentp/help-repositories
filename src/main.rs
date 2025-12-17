type AnyError = Box<dyn std::error::Error + Send + Sync>;

fn main() -> Result<(), AnyError> {
    let mut args = std::env::args();
    let program = args.next().unwrap_or_else(|| "v8_hts".to_string());

    let Some(path) = args.next() else {
        eprintln!("usage: {program} <input.vcf|input.bcf> [js_expr]");
        eprintln!("example: {program} input.vcf.gz 'variant.chrom + \":\" + variant.start'");
        return Ok(());
    };

    let js_expr = args.next().unwrap_or_else(|| "variant.start".to_string());
    v8_hts::runner::run_vcf_expr_to_stdout(&path, &js_expr, Default::default())
}
