use std::fmt;

use omp_control_client::decode_pairing_link;

pub fn decode_pairing_qr(bytes: &[u8]) -> Result<String, QrIntakeError> {
    let image = image::load_from_memory(bytes)
        .map_err(|_| QrIntakeError::InvalidImage)?
        .to_luma8();
    let mut prepared = rqrr::PreparedImage::prepare(image);
    for grid in prepared.detect_grids() {
        let Ok((_, payload)) = grid.decode() else {
            continue;
        };
        if decode_pairing_link(&payload).is_ok() {
            return Ok(payload);
        }
    }
    Err(QrIntakeError::PairingCodeNotFound)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QrIntakeError {
    InvalidImage,
    PairingCodeNotFound,
}

impl fmt::Display for QrIntakeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidImage => formatter.write_str("selected file is not a supported image"),
            Self::PairingCodeNotFound => {
                formatter.write_str("image does not contain an OMP Remote pairing code")
            }
        }
    }
}

impl std::error::Error for QrIntakeError {}

#[cfg(test)]
mod tests {
    use std::{io::Cursor, num::NonZeroU32};

    use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
    use image::{DynamicImage, ImageFormat, Luma};
    use omp_control_protocol::{
        CborCodec, PairingBundle, PairingId, PairingSecret, ServerId, TlsIdentityHint,
    };
    use qrcode::QrCode;

    use super::*;

    #[test]
    fn qr_image_round_trips_pairing_link() {
        let bundle = PairingBundle {
            format_version: 1,
            server_id: ServerId::new("server-1").unwrap(),
            endpoint: "wss://control.example.test/control".to_owned(),
            pairing_id: PairingId::new("pairing-1").unwrap(),
            secret: PairingSecret::new("secret"),
            expires_at_ms: 42,
            tls_identity: TlsIdentityHint::PubliclyTrusted,
        };
        let payload = CborCodec::new(NonZeroU32::new(64 * 1_024).unwrap())
            .encode(&bundle)
            .unwrap();
        let link = format!("omp-remote://pair#{}", URL_SAFE_NO_PAD.encode(payload));
        let qr = QrCode::new(link.as_bytes())
            .unwrap()
            .render::<Luma<u8>>()
            .min_dimensions(512, 512)
            .build();
        let mut png = Cursor::new(Vec::new());
        DynamicImage::ImageLuma8(qr)
            .write_to(&mut png, ImageFormat::Png)
            .unwrap();

        assert_eq!(decode_pairing_qr(png.get_ref()).unwrap(), link);
    }

    #[test]
    fn rejects_images_without_a_pairing_code() {
        let mut png = Cursor::new(Vec::new());
        DynamicImage::new_luma8(32, 32)
            .write_to(&mut png, ImageFormat::Png)
            .unwrap();

        assert_eq!(
            decode_pairing_qr(png.get_ref()).unwrap_err(),
            QrIntakeError::PairingCodeNotFound
        );
    }
}
