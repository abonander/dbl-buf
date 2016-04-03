use std::{mem, ptr, slice};

const CACHE_LINE: usize = 64;

pub struct AlignedBufs<T> {
    bufa: *mut T,
    bufb: *mut T,
    len: usize,
}

impl<T> AlignedBufs<T> {
    pub unsafe fn bufa(&self) -> &mut [T] {
        slice::from_raw_parts_mut(self.bufa, self.len)
    }

    pub unsafe fn bufb(&self) -> &mut [T] {
        slice::from_raw_parts_mut(self.bufb, self.len)        
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub unsafe fn drop_all(&mut self) {
        for val in self.bufa() {
            ptr::drop_in_place(val);
        }

        for val in self.bufb() {
            ptr::drop_in_place(val);
        }
    }
}

unsafe impl<T: Send> Send for AlignedBufs<T> {}
unsafe impl<T: Sync> Sync for AlignedBufs<T> {}

fn size_of_alloc<T>(len: usize) -> usize {
    mem::size_of::<T>() * len
}

fn next_cache_line(from: usize) -> usize {
    CACHE_LINE - (from & (CACHE_LINE - 1))
}

/// Return the number of elements of `T` which will fit in some multiple
/// of the cache line size.
pub fn cache_aligned_len<T>() -> usize {
    super::lcm(CACHE_LINE, mem::size_of::<T>()) / mem::size_of::<T>()
}


#[cfg(windows)]
#[allow(bad_style)]
mod imp {
    extern crate kernel32;
    extern crate winapi;

    use self::winapi::*;
   
    use std::ptr;

    use super::AlignedBufs;

    impl<T> AlignedBufs<T> {
        pub fn alloc(len: usize) -> Self {
            let bufa_bytes = super::size_of_alloc::<T>(len);
        
            let bufb_offset = super::next_cache_line(bufa_bytes);

            let total_bytes = bufb_offset + bufa_bytes;

            let bufa_ptr = unsafe { 
                let ptr = kernel32::VirtualAlloc(
                    ptr::null_mut(), 
                    total_bytes as u64, 
                    MEM_COMMIT | MEM_RESERVE,
                    PAGE_READWRITE
                );

                assert!(!ptr.is_null());

                ptr
            }; 

            let bufb_ptr = unsafe { bufa_ptr.offset(bufb_offset as isize) };

            AlignedBufs { 
                bufa: bufa_ptr as *mut T,
                bufb: bufb_ptr as *mut T,
                len: len,
            }
        }

        pub unsafe fn free(&mut self) {
            assert!(kernel32::VirtualFree(self.bufa as *mut _, 0, MEM_RELEASE) != 0);    
        }
    }
}

#[cfg(unix)]
mod imp {
}
