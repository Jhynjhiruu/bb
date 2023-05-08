use std::time::Duration;

use crate::{commands::Command, num_from_arr, BBPlayer};
use rusb::{Error, Result};

#[repr(u8)]
enum TransferCommand {
    Ready = 0x15,

    PiecemealChunkRecv = 0x1C,

    PiecemealChunkSend = 0x40,
    Ack = 0x44,

    SendChunk = 0x63,
}

impl BBPlayer {
    const READY_SIGNAL: [u8; 4] = [TransferCommand::Ready as u8, 0x00, 0x00, 0x00];

    const PIECEMEAL_DATA_CHUNK_SIZE: usize = 3;

    const TIMEOUT: Duration = Duration::SECOND;

    const PACKET_SIZE: usize = 0x80;

    const SEND_CHUNK_SIZE: usize = 0x100;

    pub fn send_chunked_data<T: AsRef<[u8]>>(&self, data: T) -> Result<()> {
        for chunk in data.as_ref().chunks(Self::SEND_CHUNK_SIZE - 2) {
            let chunk_buf = [
                &[TransferCommand::SendChunk as u8, chunk.len() as u8],
                chunk,
            ]
            .concat();
            self.bulk_transfer_send(chunk_buf, Self::TIMEOUT)?;
        }

        Ok(())
    }

    pub fn wait_ready(&self) -> Result<()> {
        while !self.is_ready()? {}
        Ok(())
    }

    fn is_ready(&self) -> Result<bool> {
        let buf = self.bulk_transfer_receive(4, Self::TIMEOUT)?;
        if buf.len() != 4 {
            Err(Error::Io)
        } else {
            Ok(buf == Self::READY_SIGNAL)
        }
    }

    fn encode_piecemeal_data(data: &[u8]) -> Vec<u8> {
        let mut rv = Vec::with_capacity(data.len() + (data.len() / 3) + (data.len() % 3).min(1));
        for chunk in data.chunks(Self::PIECEMEAL_DATA_CHUNK_SIZE) {
            rv.push(TransferCommand::PiecemealChunkSend as u8 + chunk.len() as u8);
            rv.extend(chunk);
        }
        rv
    }

    fn decode_piecemeal_data(data: &[u8], expected_len: usize) -> Result<Vec<u8>> {
        let mut buf = Vec::with_capacity(expected_len);
        let mut it = data.iter();
        while buf.len() < expected_len && let Some(&tu) = it.next() {
            match tu {
                0x1D..=0x1F => {
                    for _ in TransferCommand::PiecemealChunkRecv as u8..tu {
                        buf.push(*it.next().ok_or(Error::InvalidParam)?);
                    }
                }
                _ => return Err(Error::Io),
            }
        }
        assert!(
            buf.len() == expected_len,
            "Data length does not match expected"
        );
        Ok(buf)
    }

    pub fn send_piecemeal_data<T: AsRef<[u8]>>(&self, data: T) -> Result<usize> {
        self.bulk_transfer_send(Self::encode_piecemeal_data(data.as_ref()), Self::TIMEOUT)
    }

    pub(crate) fn send_command(&self, command: Command, arg: u32) -> Result<()> {
        self.wait_ready()?;
        let message = [(command as u32).to_be_bytes(), arg.to_be_bytes()].concat();
        match self.send_piecemeal_data(message) {
            Ok(_) => Ok(()),
            Err(e) => Err(e),
        }
    }

    fn send_ack(&self) -> Result<usize> {
        self.bulk_transfer_send([TransferCommand::Ack as u8], Self::TIMEOUT)
    }

    fn receive_data_length(&self) -> Result<usize> {
        let mut data;
        loop {
            data = self.bulk_transfer_receive(4, Self::TIMEOUT)?;
            if data == Self::READY_SIGNAL {
                eprintln!("Received unexpected ready signal");
                continue;
            }
            if data.len() != 4 || data[0] != 0x1B {
                return Err(Error::Io);
            }
            break;
        }
        Ok((num_from_arr::<u32, _>(&data) & 0x00FFFFFF) as usize)
    }

    fn receive_data(&self, expected_len: usize) -> Result<Vec<u8>> {
        let mut buf = Vec::with_capacity(
            expected_len + (expected_len / 3) + (3 - (expected_len % 3)) % 3 + 1,
        );
        let mut transferred = Self::PACKET_SIZE;

        while transferred == Self::PACKET_SIZE {
            let mut recv = self.bulk_transfer_receive(
                Self::PACKET_SIZE.min(buf.capacity() - buf.len()),
                Self::TIMEOUT,
            )?;
            transferred = recv.len();
            buf.append(&mut recv);
        }
        self.send_ack()?;
        Self::decode_piecemeal_data(&buf, expected_len)
    }

    pub fn receive_reply(&self, expected_len: usize) -> Result<Vec<u8>> {
        let data_length = self.receive_data_length()?;
        if data_length == 0 || data_length > expected_len {
            Err(Error::InvalidParam)
        } else {
            self.receive_data(data_length)
        }
    }
}
