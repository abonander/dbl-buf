use std::{mem, ptr, slice};

const CACHE_LINE: usize = 64;

pub struct AlignedBufs<T> {
    bufa: *mut T,
    bufb: *mut T,
    len: usize,
}

impl AlignedBufs<T> {
    pub unsafe fn bufa(&self) -> &mut [T] {
        slice::from_raw_parts_mut(self.bufa, self.len)
    }

    pub unsafe fn bufb(&self) -> &mut [T] {
        slice::from_raw_parts_mut(self.bufb, self.len)        
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

#[cfg(windows)]
#[allow(bad_style)]
mod imp {
    extern crate kernel32;
    extern crate winapi;

    use self::kernel32;
    use self::winapi::*;
   
    use super::AlignedBufs;

    impl AlignedBufs<T> {
        pub fn alloc(len: usize) -> Self {
            let bufa_bytes = size_of_alloc::<T>(len);
        
            let bufb_offset = next_cache_line(bufa_bytes);

            let total_bytes = bufb_offset + bufa_bytes;

            let bufa_ptr = unsafe { 
                let ptr = kernel32::VirtualAlloc(
                    ptr::null(), 
                    total_bytes, 
                    MEM_COMMIT | MEM_RESERVE,
                    MEM_READ_WRITE
                );

                assert!(!ptr.is_null());

                ptr
            }; 

            let bufb_ptr = unsafe { bufa_ptr.offset(bufb_offset as isize) };

            AlignedBufs { 
                bufa: bufa_ptr as _,
                bufb: bufb_ptr as _,
                len: len,
            }
        }

        pub unsafe fn free(&mut self) {
            assert!(kernel32::VirtualFree(self.bufa, 0, MEM_RELEASE) != 0);    
        }
    }
}

#[cfg(unix)]
mod imp {
}
