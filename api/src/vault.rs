#[cfg(windows)]
mod dpapi {
    use windows_sys::Win32::{
        Foundation::LocalFree,
        Security::Cryptography::{CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData},
    };

    fn run(input: &[u8], protect: bool) -> Option<Vec<u8>> {
        let blob_in = CRYPT_INTEGER_BLOB { cbData: input.len() as u32, pbData: input.as_ptr() as *mut u8 };
        let mut blob_out = CRYPT_INTEGER_BLOB { cbData: 0, pbData: std::ptr::null_mut() };
        let ok = unsafe {
            if protect {
                CryptProtectData(&blob_in, std::ptr::null(), std::ptr::null(), std::ptr::null(), std::ptr::null(), CRYPTPROTECT_UI_FORBIDDEN, &mut blob_out)
            } else {
                CryptUnprotectData(
                    &blob_in,
                    std::ptr::null_mut(),
                    std::ptr::null(),
                    std::ptr::null(),
                    std::ptr::null(),
                    CRYPTPROTECT_UI_FORBIDDEN,
                    &mut blob_out,
                )
            }
        };
        if ok == 0 || blob_out.pbData.is_null() {
            return None;
        }
        let out = unsafe { std::slice::from_raw_parts(blob_out.pbData, blob_out.cbData as usize).to_vec() };
        unsafe { LocalFree(blob_out.pbData.cast()) };
        Some(out)
    }

    pub fn seal(data: &[u8]) -> Option<Vec<u8>> {
        run(data, true)
    }

    pub fn open(data: &[u8]) -> Option<Vec<u8>> {
        run(data, false)
    }
}

#[cfg(not(windows))]
mod dpapi {
    pub fn seal(data: &[u8]) -> Option<Vec<u8>> {
        Some(data.to_vec())
    }

    pub fn open(data: &[u8]) -> Option<Vec<u8>> {
        Some(data.to_vec())
    }
}

pub use dpapi::{open, seal};
