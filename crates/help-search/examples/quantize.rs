//! Makes the shipped model of the help search: the weights of a model2vec
//! model as 8-bit integers, each row with its own scale (model2vec's own
//! int8 form: `embeddings` as I8 and the scales as `weights`).
//!
//! `cargo run -p sparksmith-help-search --example quantize -- IN.safetensors OUT.safetensors`
use safetensors::{Dtype, SafeTensors, tensor::TensorView};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let (Some(from), Some(to)) = (args.next(), args.next()) else {
        return Err("quantize IN.safetensors OUT.safetensors".into());
    };
    let bytes = std::fs::read(from)?;
    let tensors = SafeTensors::deserialize(&bytes)?;
    let table = tensors.tensor("embeddings")?;
    let (Dtype::F32, [rows, columns]) = (table.dtype(), table.shape()) else {
        return Err("the embeddings are not an F32 table".into());
    };
    let values: Vec<f32> = table
        .data()
        .chunks_exact(4)
        .map(|b| b.try_into().map(f32::from_le_bytes))
        .collect::<Result<_, _>>()?;
    let mut small = Vec::with_capacity(values.len());
    let mut scales = Vec::with_capacity(rows * 4);
    for row in values.chunks_exact((*columns).max(1)) {
        let largest = row.iter().fold(0.0_f32, |m, v| m.max(v.abs()));
        let scale = if largest > 0.0 { largest / 127.0 } else { 1.0 };
        small.extend(
            row.iter()
                .map(|v| (v / scale).round().clamp(-127.0, 127.0) as i8 as u8),
        );
        scales.extend(scale.to_le_bytes());
    }
    let out = [
        (
            "embeddings",
            TensorView::new(Dtype::I8, vec![*rows, *columns], &small)?,
        ),
        (
            "weights",
            TensorView::new(Dtype::F32, vec![*rows], &scales)?,
        ),
    ];
    std::fs::write(to, safetensors::serialize(out, &None)?)?;
    Ok(())
}
