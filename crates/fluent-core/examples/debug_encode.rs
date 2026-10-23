use std::path::Path;

use fluent_core::NllbTokenizer;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let dir = Path::new("models/nllb-200-1.3b");
    let tokenizer = NllbTokenizer::from_dir(dir)?;
    let text = std::env::args().nth(1).unwrap_or_else(|| {
        "Hello, how are you today? I am testing a local translation model.".into()
    });
    let (ids, truncated) = tokenizer.encode_source(&text, "eng_Latn", 512)?;
    println!("truncated={truncated} len={}", ids.len());
    println!("ids={ids:?}");
    println!("eos={} pad={}", tokenizer.eos_id(), tokenizer.pad_id());
    for id in &ids {
        println!(
            "{id:>7}  {:?}",
            tokenizer
                .token_to_id("")
                .map(|_| String::new())
                .unwrap_or_default()
        );
    }
    println!("decoded source back: {:?}", tokenizer.decode(&ids)?);
    let one = tokenizer.encode_source("Hello, how are you today?", "eng_Latn", 512)?;
    println!("first sentence: {:?}", one.0);
    println!("first decoded: {:?}", tokenizer.decode(&one.0)?);
    Ok(())
}
