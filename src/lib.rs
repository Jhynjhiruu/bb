#![feature(duration_constants)]
#![feature(split_array)]
#![feature(let_chains)]

use chrono::prelude::*;
use commands::BlockSpare;
use std::mem::size_of;

use error::{LibBBError, Result};
use fs::FSBlock;
use rusb::{Device, DeviceHandle, DeviceList, GlobalContext};

pub(crate) mod commands;
pub(crate) mod constants;
pub mod error;
mod fs;
mod player_comms;
mod usb;

#[derive(Debug)]
pub struct BBPlayer {
    handle: DeviceHandle<GlobalContext>,
    current_fs_index: u32,
    current_fs_block: Option<FSBlock>,
    current_fs_spare: Vec<u8>,
    is_initialised: bool,
}

trait FromBE {
    fn from_be_bytes(data: [u8; 4]) -> Self;
}

macro_rules! from_be {
    ($($t:ty)+) => {
        $(impl FromBE for $t {
            fn from_be_bytes(data: [u8; 4]) -> Self {
                Self::from_be_bytes(data)
            }
        })+
    };
}

from_be!(u32 i32);

macro_rules! check_initialised {
    ($e:expr, $b:block) => {
        if $e $b else { Err(LibBBError::NoConsole) }
    };
}

fn num_from_arr<T: FromBE, U: AsRef<[u8]>>(data: U) -> T {
    assert!(data.as_ref().len() == size_of::<T>());
    match data.as_ref() {
        &[b0, b1, b2, b3] => T::from_be_bytes([b0, b1, b2, b3]),
        _ => unreachable!(),
    }
}

impl BBPlayer {
    pub fn get_players() -> Result<Vec<Device<GlobalContext>>> {
        let devices = DeviceList::new()?;
        let mut rv = vec![];

        for device in devices.iter() {
            if Self::is_bbp(&device)? {
                rv.push(device);
            }
        }

        Ok(rv)
    }

    pub fn new(device: &Device<GlobalContext>) -> Result<Self> {
        Ok(Self {
            handle: Self::open_device(device)?,
            current_fs_index: 0,
            current_fs_block: None,
            current_fs_spare: vec![],
            is_initialised: false,
        })
    }

    pub fn initialised(&self) -> bool {
        self.is_initialised
    }

    #[allow(non_snake_case)]
    pub fn Init(&mut self) -> Result<()> {
        self.set_seqno(0x01)?;
        self.get_num_blocks()?;
        if !self.get_current_fs()? {
            return Err(LibBBError::FS);
        }
        self.init_fs()?;
        self.delete_file_and_update("temp.tmp")?;
        self.is_initialised = true;
        Ok(())
    }

    #[allow(non_snake_case)]
    pub fn GetBBID(&self) -> Result<u32> {
        check_initialised!(self.is_initialised, { self.get_bbid() })
    }

    #[allow(non_snake_case)]
    pub fn SetLED(&self, ledval: u32) -> Result<()> {
        check_initialised!(self.is_initialised, { self.set_led(ledval) })
    }

    // signhash

    #[allow(non_snake_case)]
    pub fn SetTime<Tz: TimeZone>(&self, when: DateTime<Tz>) -> Result<()> {
        check_initialised!(self.is_initialised, {
            let timedata = [
                (when.year() % 100) as u8,
                when.month() as u8,
                when.day() as u8,
                when.weekday() as u8,
                0,
                when.hour() as u8,
                when.minute() as u8,
                when.second() as u8,
            ];

            self.set_time(timedata)
        })
    }

    #[allow(non_snake_case)]
    pub fn ListFileBlocks<T: AsRef<str>>(&self, filename: T) -> Result<Option<Vec<u16>>> {
        check_initialised!(self.is_initialised, {
            self.list_file_blocks(filename.as_ref())
        })
    }

    #[allow(non_snake_case)]
    pub fn ListFiles(&self) -> Result<Vec<(String, u32)>> {
        check_initialised!(self.is_initialised, { self.list_files() })
    }

    #[allow(non_snake_case)]
    pub fn DumpCurrentFS(&self) -> Result<Vec<u8>> {
        check_initialised!(self.is_initialised, { self.dump_current_fs() })
    }

    #[allow(non_snake_case)]
    pub fn DumpNAND(&self) -> Result<BlockSpare> {
        check_initialised!(self.is_initialised, { self.dump_nand_and_spare() })
    }

    #[allow(non_snake_case)]
    pub fn ReadSingleBlock(&self, block_num: u32) -> Result<BlockSpare> {
        check_initialised!(self.is_initialised, { self.read_single_block(block_num) })
    }

    // WriteNAND

    #[allow(non_snake_case)]
    pub fn WriteSingleBlock<T: AsRef<[u8]>, U: AsRef<[u8]>>(
        &self,
        block: T,
        spare: U,
        block_num: u32,
    ) -> Result<()> {
        check_initialised!(self.is_initialised, {
            self.write_single_block(block.as_ref(), spare.as_ref(), block_num)
        })
    }

    #[allow(non_snake_case)]
    pub fn ReadFile<T: AsRef<str>>(&self, filename: T) -> Result<Option<Vec<u8>>> {
        check_initialised!(self.is_initialised, { self.read_file(filename.as_ref()) })
    }

    #[allow(non_snake_case)]
    pub fn WriteFile<T: AsRef<[u8]>, U: AsRef<str>>(&mut self, data: T, filename: U) -> Result<()> {
        check_initialised!(self.is_initialised, {
            self.write_file(data.as_ref(), filename.as_ref())
        })
    }

    #[allow(non_snake_case)]
    pub fn DeleteFile<T: AsRef<str>>(&mut self, filename: T) -> Result<()> {
        check_initialised!(self.is_initialised, {
            self.delete_file_and_update(filename.as_ref())
        })
    }

    #[allow(non_snake_case)]
    pub fn GetStats(&self) -> Result<(usize, usize, usize, u32)> {
        check_initialised!(self.is_initialised, { self.get_stats() })
    }

    #[allow(non_snake_case)]
    pub fn Close(&mut self) -> Result<()> {
        check_initialised!(self.is_initialised, {
            match self.close_connection() {
                Ok(_) => {}
                Err(e) => return Err(e),
            }
            self.is_initialised = false;
            Ok(())
        })
    }
}

impl Drop for BBPlayer {
    fn drop(&mut self) {
        if self.is_initialised {
            match self.close_connection() {
                Ok(_) => {}
                Err(e) => {
                    eprintln!("{e}");
                    return;
                }
            }
            self.is_initialised = false;
        }
    }
}
