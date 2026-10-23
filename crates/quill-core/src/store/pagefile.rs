//! A page-encrypted file that redb uses as its storage.
//!
//! redb sees an ordinary byte array; this file stores it as 4 KiB pages,
//! each sealed with XChaCha20-Poly1305 under the store's page key. A thief
//! with the file learns its size and, given several copies, which pages
//! changed; nothing about tables, keys or values.
//!
//! **Atomic pages.** Every page has two slots on disk. A write goes to the
//! slot not holding the page's current contents, with a counter one higher;
//! a read takes the highest-counter slot that authenticates. A crash mid-
//! write therefore leaves the page as it was (the torn slot fails
//! authentication), which is stronger than a disk gives redb. This doubles
//! the file size, which suits a client store.
//!
//! **Binding.** Each slot's associated data is the file id, the page number
//! and the counter, so a slot can't be moved to another page, another
//! store, or swapped between the two slots of one page. Replaying an older
//! copy of a whole slot (rollback) is not detected, as with SQLCipher.
//!
//! Physical layout: the header region ([`HEADER_REGION`] bytes), then for
//! each page two slots of `counter (8) | nonce (24) | ciphertext (4096) |
//! tag (16)`. An all-zero slot (counter 0) is empty; a page with both slots
//! empty reads as zeros, so growing the file is a plain zero-filled extend.

use std::{
    collections::HashMap,
    fs::File,
    io::{self, Read, Seek, SeekFrom, Write},
    sync::Mutex,
};

use chacha20poly1305::{AeadInOut, KeyInit, Tag, XChaCha20Poly1305, XNonce};
use redb::StorageBackend;
use zeroize::Zeroizing;

use super::header::HEADER_REGION;

/// Bytes of plaintext per page, as redb sees them.
pub(crate) const PAGE: usize = 4096;
const COUNTER: usize = 8;
const NONCE: usize = 24;
const TAG: usize = 16;
const SLOT: usize = COUNTER + NONCE + PAGE + TAG;
const PAGE_SPAN: u64 = 2 * SLOT as u64;
const PAGE_U64: u64 = PAGE as u64;

/// Which slot holds a page's current contents, and its counter.
#[derive(Debug, Clone, Copy)]
struct Current {
    slot: usize,
    counter: u64,
}

struct Inner {
    file: File,
    pages: u64,
    /// Pages whose current slot is known, to skip a read before writing.
    current: HashMap<u64, Current>,
}

pub(crate) struct PageFile {
    inner: Mutex<Inner>,
    cipher: XChaCha20Poly1305,
    file_id: [u8; 16],
}

impl std::fmt::Debug for PageFile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PageFile").finish_non_exhaustive()
    }
}

fn corrupt(msg: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg.into())
}

impl PageFile {
    /// Wraps an open, locked store file whose header is already written.
    pub fn new(mut file: File, page_key: &[u8; 32], file_id: [u8; 16]) -> io::Result<Self> {
        let len = file.seek(SeekFrom::End(0))?;
        let body = len.saturating_sub(HEADER_REGION);
        // A crash while growing can leave a partial page: round it up.
        let pages = body.div_ceil(PAGE_SPAN);
        if body % PAGE_SPAN != 0 {
            file.set_len(HEADER_REGION + pages * PAGE_SPAN)?;
        }
        Ok(Self {
            inner: Mutex::new(Inner {
                file,
                pages,
                current: HashMap::new(),
            }),
            cipher: XChaCha20Poly1305::new(page_key.into()),
            file_id,
        })
    }

    fn lock(&self) -> io::Result<std::sync::MutexGuard<'_, Inner>> {
        self.inner
            .lock()
            .map_err(|_| io::Error::other("store file lock poisoned"))
    }

    /// Runs `f` with the underlying file (for header updates).
    pub fn with_file<T>(&self, f: impl FnOnce(&mut File) -> io::Result<T>) -> io::Result<T> {
        f(&mut self.lock()?.file)
    }

    fn aad(&self, page: u64, counter: u64) -> [u8; 32] {
        let mut aad = [0u8; 32];
        aad[..16].copy_from_slice(&self.file_id);
        aad[16..24].copy_from_slice(&page.to_be_bytes());
        aad[24..].copy_from_slice(&counter.to_be_bytes());
        aad
    }

    fn slot_offset(page: u64, slot: usize) -> u64 {
        HEADER_REGION + page * PAGE_SPAN + if slot == 0 { 0 } else { SLOT as u64 }
    }

    /// Decrypts a page, returning its contents and current slot (none if
    /// the page has never been written).
    fn load(
        &self,
        inner: &mut Inner,
        page: u64,
    ) -> io::Result<(Zeroizing<Vec<u8>>, Option<Current>)> {
        let mut raw = Zeroizing::new(vec![0u8; 2 * SLOT]);
        inner
            .file
            .seek(SeekFrom::Start(Self::slot_offset(page, 0)))?;
        inner.file.read_exact(&mut raw)?;
        let mut slots: Vec<(usize, u64)> = (0..2)
            .map(|i| {
                let mut c = [0u8; COUNTER];
                c.copy_from_slice(&raw[i * SLOT..i * SLOT + COUNTER]);
                (i, u64::from_be_bytes(c))
            })
            .filter(|(_, counter)| *counter != 0)
            .collect();
        if slots.is_empty() {
            inner.current.remove(&page);
            return Ok((Zeroizing::new(vec![0u8; PAGE]), None));
        }
        slots.sort_by_key(|(_, counter)| std::cmp::Reverse(*counter));
        for (slot, counter) in slots {
            let base = slot * SLOT;
            let nonce = XNonce::try_from(&raw[base + COUNTER..base + COUNTER + NONCE])
                .map_err(|_| corrupt("bad nonce"))?;
            let tag_at = base + COUNTER + NONCE + PAGE;
            let tag = Tag::try_from(&raw[tag_at..tag_at + TAG]).map_err(|_| corrupt("bad tag"))?;
            let mut plain = Zeroizing::new(raw[base + COUNTER + NONCE..tag_at].to_vec());
            let opened = self.cipher.decrypt_inout_detached(
                &nonce,
                &self.aad(page, counter),
                plain.as_mut_slice().into(),
                &tag,
            );
            if opened.is_ok() {
                let current = Current { slot, counter };
                inner.current.insert(page, current);
                return Ok((plain, Some(current)));
            }
        }
        Err(corrupt(format!(
            "store page {page} failed authentication: the file is damaged or was altered"
        )))
    }

    /// Encrypts `plain` as the page's new contents, into its other slot.
    fn store(
        &self,
        inner: &mut Inner,
        page: u64,
        plain: &[u8],
        prev: Option<Current>,
    ) -> io::Result<()> {
        let (slot, counter) = match prev {
            Some(c) => (1 - c.slot, c.counter + 1),
            None => (0, 1),
        };
        let mut nonce = [0u8; NONCE];
        super::random(&mut nonce).map_err(|e| io::Error::other(e.to_string()))?;
        let mut out = Zeroizing::new(vec![0u8; SLOT]);
        out[..COUNTER].copy_from_slice(&counter.to_be_bytes());
        out[COUNTER..COUNTER + NONCE].copy_from_slice(&nonce);
        let body = &mut out[COUNTER + NONCE..COUNTER + NONCE + PAGE];
        body.copy_from_slice(plain);
        let tag = self
            .cipher
            .encrypt_inout_detached(&XNonce::from(nonce), &self.aad(page, counter), body.into())
            .map_err(|_| io::Error::other("page encryption failed"))?;
        out[COUNTER + NONCE + PAGE..].copy_from_slice(&tag);
        inner
            .file
            .seek(SeekFrom::Start(Self::slot_offset(page, slot)))?;
        inner.file.write_all(&out)?;
        inner.current.insert(page, Current { slot, counter });
        Ok(())
    }

    fn check_range(inner: &Inner, offset: u64, len: usize) -> io::Result<()> {
        let end = offset
            .checked_add(len as u64)
            .ok_or_else(|| corrupt("range overflows"))?;
        if end > inner.pages * PAGE_U64 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "read or write past the end of the store",
            ));
        }
        Ok(())
    }
}

impl StorageBackend for PageFile {
    fn len(&self) -> Result<u64, io::Error> {
        Ok(self.lock()?.pages * PAGE_U64)
    }

    fn read(&self, offset: u64, out: &mut [u8]) -> Result<(), io::Error> {
        let mut inner = self.lock()?;
        Self::check_range(&inner, offset, out.len())?;
        let mut done = 0usize;
        while done < out.len() {
            let pos = offset + done as u64;
            let page = pos / PAGE_U64;
            let within = usize::try_from(pos % PAGE_U64).unwrap_or(0);
            let take = (PAGE - within).min(out.len() - done);
            let (plain, _) = self.load(&mut inner, page)?;
            out[done..done + take].copy_from_slice(&plain[within..within + take]);
            done += take;
        }
        Ok(())
    }

    fn set_len(&self, len: u64) -> Result<(), io::Error> {
        let mut inner = self.lock()?;
        let pages = len.div_ceil(PAGE_U64);
        if pages < inner.pages {
            inner.file.set_len(HEADER_REGION + pages * PAGE_SPAN)?;
            inner.current.retain(|p, _| *p < pages);
        }
        let old_pages = inner.pages;
        inner.pages = pages;
        if pages > old_pages {
            // New pages are empty slots, which read as zeros.
            inner.file.set_len(HEADER_REGION + pages * PAGE_SPAN)?;
        }
        // Positions past `len` in its last page must read as zero if the
        // store grows again.
        let tail = usize::try_from(len % PAGE_U64).unwrap_or(0);
        if tail != 0 && pages <= old_pages {
            let page = pages - 1;
            let (mut plain, prev) = self.load(&mut inner, page)?;
            if plain[tail..].iter().any(|b| *b != 0) {
                plain[tail..].fill(0);
                self.store(&mut inner, page, &plain, prev)?;
            }
        }
        Ok(())
    }

    fn sync_data(&self) -> Result<(), io::Error> {
        self.lock()?.file.sync_data()
    }

    fn write(&self, offset: u64, data: &[u8]) -> Result<(), io::Error> {
        let mut inner = self.lock()?;
        Self::check_range(&inner, offset, data.len())?;
        let mut done = 0usize;
        while done < data.len() {
            let pos = offset + done as u64;
            let page = pos / PAGE_U64;
            let within = usize::try_from(pos % PAGE_U64).unwrap_or(0);
            let take = (PAGE - within).min(data.len() - done);
            if take == PAGE {
                // A whole page: only the current slot is needed, not its contents.
                let prev = match inner.current.get(&page).copied() {
                    Some(c) => Some(c),
                    None => self.load(&mut inner, page)?.1,
                };
                self.store(&mut inner, page, &data[done..done + PAGE], prev)?;
            } else {
                let (mut plain, prev) = self.load(&mut inner, page)?;
                plain[within..within + take].copy_from_slice(&data[done..done + take]);
                self.store(&mut inner, page, &plain, prev)?;
            }
            done += take;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file_with_header() -> (tempfile::TempDir, File) {
        let dir = tempfile::tempdir().unwrap();
        let mut f = File::options()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(dir.path().join("pages"))
            .unwrap();
        f.write_all(&[0u8; HEADER_REGION as usize]).unwrap();
        (dir, f)
    }

    fn reopen(dir: &tempfile::TempDir) -> File {
        File::options()
            .read(true)
            .write(true)
            .open(dir.path().join("pages"))
            .unwrap()
    }

    #[test]
    fn reads_back_writes_across_pages_and_grows_with_zeros() {
        let (dir, f) = file_with_header();
        let pf = PageFile::new(f, &[7; 32], [1; 16]).unwrap();
        pf.set_len(3 * PAGE_U64).unwrap();
        let mut buf = vec![1u8; 3 * PAGE];
        pf.read(0, &mut buf).unwrap();
        assert!(buf.iter().all(|b| *b == 0));
        let data: Vec<u8> = (0..6000u32).map(|i| (i % 251) as u8).collect();
        pf.write(3000, &data).unwrap();
        let mut back = vec![0u8; data.len()];
        pf.read(3000, &mut back).unwrap();
        assert_eq!(back, data);
        assert!(pf.read(3 * PAGE_U64 - 1, &mut [0u8; 2]).is_err());
        drop(pf);

        // Reopened with the same key: same contents. With another: refused.
        let pf = PageFile::new(reopen(&dir), &[7; 32], [1; 16]).unwrap();
        assert_eq!(pf.len().unwrap(), 3 * PAGE_U64);
        let mut back = vec![0u8; data.len()];
        pf.read(3000, &mut back).unwrap();
        assert_eq!(back, data);
        let wrong = PageFile::new(reopen(&dir), &[8; 32], [1; 16]).unwrap();
        assert!(wrong.read(3000, &mut back).is_err());
        let moved = PageFile::new(reopen(&dir), &[7; 32], [2; 16]).unwrap();
        assert!(moved.read(3000, &mut back).is_err());
    }

    #[test]
    fn plaintext_never_reaches_the_file() {
        let (dir, f) = file_with_header();
        let pf = PageFile::new(f, &[7; 32], [1; 16]).unwrap();
        pf.set_len(PAGE_U64).unwrap();
        let marker = b"CANARY-plaintext-marker-CANARY";
        pf.write(100, marker).unwrap();
        pf.write(200, marker).unwrap();
        drop(pf);
        let raw = std::fs::read(dir.path().join("pages")).unwrap();
        assert!(!raw.windows(marker.len()).any(|w| w == marker));
    }

    #[test]
    fn a_torn_write_leaves_the_previous_contents() {
        let (dir, f) = file_with_header();
        let pf = PageFile::new(f, &[7; 32], [1; 16]).unwrap();
        pf.set_len(PAGE_U64).unwrap();
        pf.write(0, &[1u8; PAGE]).unwrap();
        pf.write(0, &[2u8; PAGE]).unwrap();
        drop(pf);
        // Tear the newest write (slot 1, counter 2): damage its ciphertext.
        let mut f = reopen(&dir);
        f.seek(SeekFrom::Start(HEADER_REGION + SLOT as u64 + 500))
            .unwrap();
        f.write_all(&[0xAA; 100]).unwrap();
        let pf = PageFile::new(f, &[7; 32], [1; 16]).unwrap();
        let mut buf = vec![0u8; PAGE];
        pf.read(0, &mut buf).unwrap();
        assert!(buf.iter().all(|b| *b == 1));
        // The next write replaces the torn slot and wins.
        pf.write(0, &[3u8; PAGE]).unwrap();
        pf.read(0, &mut buf).unwrap();
        assert!(buf.iter().all(|b| *b == 3));
        drop(pf);
        let pf = PageFile::new(reopen(&dir), &[7; 32], [1; 16]).unwrap();
        pf.read(0, &mut buf).unwrap();
        assert!(buf.iter().all(|b| *b == 3));
    }

    #[test]
    fn swapped_or_altered_slots_are_refused() {
        let (dir, f) = file_with_header();
        let pf = PageFile::new(f, &[7; 32], [1; 16]).unwrap();
        pf.set_len(2 * PAGE_U64).unwrap();
        pf.write(0, &[1u8; PAGE]).unwrap();
        pf.write(PAGE_U64, &[2u8; PAGE]).unwrap();
        drop(pf);
        // Copy page 1's slot over page 0's: bound to page 1, so refused.
        let mut raw = std::fs::read(dir.path().join("pages")).unwrap();
        let h = HEADER_REGION as usize;
        let p1 = h + PAGE_SPAN as usize;
        let slot1: Vec<u8> = raw[p1..p1 + SLOT].to_vec();
        raw[h..h + SLOT].copy_from_slice(&slot1);
        std::fs::write(dir.path().join("pages"), &raw).unwrap();
        let pf = PageFile::new(reopen(&dir), &[7; 32], [1; 16]).unwrap();
        assert!(pf.read(0, &mut [0u8; 16]).is_err());
        let mut ok = [0u8; 16];
        pf.read(PAGE_U64, &mut ok).unwrap();
        assert_eq!(ok, [2u8; 16]);
    }

    #[test]
    fn shrinking_zeroes_the_tail_of_the_last_page() {
        let (_dir, f) = file_with_header();
        let pf = PageFile::new(f, &[7; 32], [1; 16]).unwrap();
        pf.set_len(2 * PAGE_U64).unwrap();
        pf.write(0, &[9u8; 2 * PAGE]).unwrap();
        pf.set_len(100).unwrap();
        assert_eq!(pf.len().unwrap(), PAGE_U64);
        pf.set_len(2 * PAGE_U64).unwrap();
        let mut buf = vec![0u8; 2 * PAGE];
        pf.read(0, &mut buf).unwrap();
        assert!(buf[..100].iter().all(|b| *b == 9));
        assert!(buf[100..].iter().all(|b| *b == 0));
    }
}
