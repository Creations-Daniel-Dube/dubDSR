use std::fs::File;
use std::io::BufWriter;
use std::path::Path;

use tiff::encoder::colortype::{Gray16, RGB16};
use tiff::encoder::TiffEncoder;

use crate::error::PipelineError;

pub fn write_tiff(path: &Path, width: u32, height: u32, rgb: bool, data: &[u16]) -> Result<(), PipelineError> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent).map_err(|err| PipelineError::with("error.write", err.to_string()))?;
    }
    let file = File::create(path).map_err(|err| PipelineError::with("error.write", err.to_string()))?;
    let mut encoder = TiffEncoder::new(BufWriter::new(file)).map_err(|err| PipelineError::with("error.write", err.to_string()))?;
    if rgb {
        encoder
            .write_image::<RGB16>(width, height, data)
            .map_err(|err| PipelineError::with("error.write", err.to_string()))?;
    } else {
        encoder
            .write_image::<Gray16>(width, height, data)
            .map_err(|err| PipelineError::with("error.write", err.to_string()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;

    #[test]
    fn writes_gray16_roundtrip() {
        let dir = std::env::temp_dir().join(format!("dubdsr-tiff-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("sample.tiff");
        let data = vec![0u16, 257, 65535, 1000];
        write_tiff(&path, 2, 2, false, &data).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        let mut decoder = tiff::decoder::Decoder::new(Cursor::new(bytes)).unwrap();
        let (width, height) = decoder.dimensions().unwrap();
        assert_eq!((width, height), (2, 2));
        let image = decoder.read_image().unwrap();
        match image {
            tiff::decoder::DecodingResult::U16(values) => assert_eq!(values, data),
            other => panic!("résultat inattendu: {other:?}"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
