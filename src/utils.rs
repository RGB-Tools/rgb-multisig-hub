use std::{
    net::{SocketAddr, TcpStream},
    path::Path,
};

use sha2::{Digest, Sha256};
use tempfile::NamedTempFile;
use time::OffsetDateTime;
use tokio::{fs::File, io::AsyncReadExt};

use crate::{
    error::{APIError, AppError},
    routes::OperationType,
};

pub(crate) fn check_port_is_available(port: u16) -> Result<(), AppError> {
    if TcpStream::connect(SocketAddr::from(([127, 0, 0, 1], port))).is_ok() {
        return Err(AppError::UnavailablePort(port));
    }
    Ok(())
}

pub(crate) async fn no_cancel<Fut>(fut: Fut) -> Fut::Output
where
    Fut: 'static + Future + Send,
    Fut::Output: Send,
{
    let (tx, rx) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        let result = fut.await;
        let _ = tx.send(result);
    });
    rx.await.unwrap()
}

pub(crate) fn hex_str_to_vec(hex: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(hex.len() / 2);
    let mut b = 0;
    for (idx, c) in hex.as_bytes().iter().enumerate() {
        b <<= 4;
        match *c {
            b'A'..=b'F' => b |= c - b'A' + 10,
            b'a'..=b'f' => b |= c - b'a' + 10,
            b'0'..=b'9' => b |= c - b'0',
            _ => return None,
        }
        if (idx & 1) == 1 {
            out.push(b);
            b = 0;
        }
    }
    Some(out)
}

pub(crate) fn now() -> OffsetDateTime {
    OffsetDateTime::now_utc()
}

pub(crate) async fn compute_file_id(path: &Path) -> Result<String, APIError> {
    let mut file = File::open(path).await?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 8192];
    loop {
        let n = file.read(&mut buffer).await?;
        if n == 0 {
            break;
        }
        hasher.update(&buffer[..n]);
    }
    Ok(hex::encode(hasher.finalize()))
}

pub(crate) fn get_threshold_for_operation(
    op_type: &OperationType,
    threshold_vanilla: u8,
    threshold_colored: u8,
) -> Option<u8> {
    match op_type {
        OperationType::CreateUtxos | OperationType::SendBtc => Some(threshold_vanilla),
        OperationType::SendRgb | OperationType::Inflation => Some(threshold_colored),
        OperationType::Issuance | OperationType::BlindReceive | OperationType::WitnessReceive => {
            None
        }
    }
}

pub(crate) async fn persist_temp_file(
    temp_file: NamedTempFile,
    file_path: &Path,
) -> Result<(), APIError> {
    let persisted = temp_file
        .persist(file_path)
        .map_err(|e| APIError::Unexpected(format!("failed to persist file: {e}")))?;
    persisted
        .sync_all()
        .map_err(|e| APIError::Unexpected(format!("failed to sync file: {e}")))?;
    if let Some(parent) = file_path.parent() {
        let dir = File::open(parent)
            .await
            .map_err(|e| APIError::Unexpected(format!("failed to open directory: {e}")))?;
        dir.sync_all().await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::routes::AUTO_APPROVED_OPS;

    use super::*;

    #[test]
    fn test_get_threshold_for_operation() {
        let threshold_vanilla = 1;
        let threshold_colored = 2;
        for op_type in AUTO_APPROVED_OPS {
            assert_eq!(
                get_threshold_for_operation(&op_type, threshold_vanilla, threshold_colored),
                None
            );
        }
        assert_eq!(
            get_threshold_for_operation(
                &OperationType::CreateUtxos,
                threshold_vanilla,
                threshold_colored
            ),
            Some(threshold_vanilla)
        );
        assert_eq!(
            get_threshold_for_operation(
                &OperationType::SendBtc,
                threshold_vanilla,
                threshold_colored
            ),
            Some(threshold_vanilla)
        );
        assert_eq!(
            get_threshold_for_operation(
                &OperationType::SendRgb,
                threshold_vanilla,
                threshold_colored
            ),
            Some(threshold_colored)
        );
        assert_eq!(
            get_threshold_for_operation(
                &OperationType::Inflation,
                threshold_vanilla,
                threshold_colored
            ),
            Some(threshold_colored)
        );
    }
}
