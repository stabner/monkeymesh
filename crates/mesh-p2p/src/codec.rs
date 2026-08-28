//! Bincode codec for libp2p request-response.

use std::io;

use async_trait::async_trait;
use futures::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use libp2p::request_response::Codec;
use libp2p::StreamProtocol;

use crate::protocol::{NetMsg, MESH_RR_PROTOCOL};

const MAX_FRAME: u32 = 16 * 1024 * 1024;

#[derive(Clone, Default)]
pub struct MeshCodec;

#[async_trait]
impl Codec for MeshCodec {
    type Protocol = StreamProtocol;
    type Request = NetMsg;
    type Response = NetMsg;

    async fn read_request<T>(&mut self, _: &Self::Protocol, io: &mut T) -> io::Result<Self::Request>
    where
        T: AsyncRead + Unpin + Send,
    {
        read_framed(io).await
    }

    async fn read_response<T>(
        &mut self,
        _: &Self::Protocol,
        io: &mut T,
    ) -> io::Result<Self::Response>
    where
        T: AsyncRead + Unpin + Send,
    {
        read_framed(io).await
    }

    async fn write_request<T>(
        &mut self,
        _: &Self::Protocol,
        io: &mut T,
        req: Self::Request,
    ) -> io::Result<()>
    where
        T: AsyncWrite + Unpin + Send,
    {
        write_framed(io, &req).await
    }

    async fn write_response<T>(
        &mut self,
        _: &Self::Protocol,
        io: &mut T,
        res: Self::Response,
    ) -> io::Result<()>
    where
        T: AsyncWrite + Unpin + Send,
    {
        write_framed(io, &res).await
    }
}

pub fn protocol_name() -> StreamProtocol {
    StreamProtocol::new(MESH_RR_PROTOCOL)
}

async fn read_framed<T: AsyncRead + Unpin>(io: &mut T) -> io::Result<NetMsg> {
    let mut len_buf = [0u8; 4];
    io.read_exact(&mut len_buf).await?;
    let len = u32::from_le_bytes(len_buf);
    if len == 0 || len > MAX_FRAME {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("bad frame length {len}"),
        ));
    }
    let mut buf = vec![0u8; len as usize];
    io.read_exact(&mut buf).await?;
    bincode::deserialize(&buf).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

async fn write_framed<T: AsyncWrite + Unpin>(io: &mut T, msg: &NetMsg) -> io::Result<()> {
    let payload =
        bincode::serialize(msg).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    if payload.len() > MAX_FRAME as usize {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "message too large",
        ));
    }
    io.write_all(&(payload.len() as u32).to_le_bytes()).await?;
    io.write_all(&payload).await?;
    io.close().await?;
    Ok(())
}
