use math;

use std::marker::PhantomData;
use std::{mem, ptr, slice};

const CACHE_LINE: usize = 64;

pub struct Bufs<T> {
    alloc: Box<[u8]>,
    bufa_offset: usize,
    bufb_offset: usize,
    len: usize,
    _marker: PhantomData<T>,
}

impl<T> Bufs<T> {
    pub fn alloc(cap: usize) -> Self {
        let (alloc_size, bufb_offset) = alloc_info::<T>(cap);
        
        let alloc = {
            let mut vec = Vec::with_capacity(alloc_size);
            unsafe {
                vec.set_len(alloc_size);
            }

            vec.into_boxed_slice()
        };

        let bufa_offset = aligned_offset(alloc.as_ptr());
        let bufb_offset = bufa_offset + bufb_offset;

        Bufs {
            alloc: alloc,
            bufa_offset: bufa_offset,
            bufb_offset: bufb_offset,
            len: 0,
        }    
    }

    pub fn with(mut bufa: Vec<T>, mut bufb: Vec<T>) -> Self {
        let len = cmp::min(bufa.len(), bufb.len());
        bufa.truncate(len);
        bufb.truncate(len);

        let mut self_ = Self::alloc(len);

        unsafe {
            self_.set_len(len);
        }

        let (bufa_, bufb_) = self_.bufs();

        unsafe {
            ptr::copy_nonoverlapping(
                bufa.as_ptr(),
                bufa_.as_mut_ptr(),
                len
            );

            ptr::copy_nonoverlapping(
                bufb.as_ptr(),
                bufb_.as_mut_ptr(),
                len
            );
        }

        self_
    }


    pub fn realloc(&mut self, new_cap: usize) {
        let old_offset = self.bufb_offset();
        let old_size = self.alloc_size();

        let (new_offset, new_size) = alloc_info::<T>(new_len);        

        unsafe {
            // Move before the resize if it's going to shrink
            if new_len < self.len { 
                self.move_bufb(new_offset); 
            }

            let ptr = heap::reallocate(
                self.bufa as *mut u8,
                old_size,
                new_size,
                CACHE_LINE
            );

            assert!(!ptr.is_null(), "Failure to reallocate");

            self.bufa = ptr as *mut T;

            // Move after the resize if it's going to grow
            if new_len > self.len {
                self.bufb = ptr.offset(old_offset as isize) as *mut T;
                self.move_bufb(new_offset);
            } else {
                // Fixup the bufb pointer
                self.bufb = ptr.offset(new_offset as isize) as *mut T;
            }
        }
 
        self.len = new_len;
    }

    pub fn bufs(&mut self) -> (&mut [T], &mut [T]) {
        unsafe {
            (
                slice::from_parts_mut(
                    (&mut self.alloc[self.bufa_offset]) as *mut u8 as *mut T,
                    self.len
                ),
                slice::from_parts_mut(
                    (&mut self.alloc[self.bufb_offset]) as *mut u8 as *mut T,
                    self.len
                )
            )
        }
    }

    pub unsafe fn set_len(&mut self, len: usize) {
        self.len = len;
    }

    pub fn len(&self) -> usize {
        self.len
    } 

    unsafe fn move_bufb(&mut self, new_offset: usize) {
        let bufb_ptr = self.bufs().1.as_mut_ptr();
        
        let curr_offset = self.bufb_offset;
        let rel_offset = new_offset as isize - (curr_offset as isize);
        
        let bufb_newptr = (bufb_ptr as *mut u8).offset(rel_offset) as *mut T;
        ptr::copy(bufb_ptr, bufb_newptr, self.len);

        self.bufb_offset = new_offset;
    }

    fn bufb_offset(&self) -> usize {
        self.bufb as usize - (self.bufa as usize)
    }

    fn alloc_size(&self) -> usize {
        self.bufb as usize + self.bufb_offset()
    }
}

fn size_of_alloc<T>(len: usize) -> usize {
    mem::size_of::<T>().checked_mul(len)
        .expect("Overflow calculating alloc size")
}

fn aligned_offset<T>(ptr: *const T) -> usize {
    let ptr_val = ptr as usize;
    let aligned_ptr = next_cache_line(ptr_val);
    aligned_ptr - ptr_val
}    

// Get the next multiple of `CACHE_LINE` up from `from`
fn next_cache_line(from: usize) -> usize {
    math::next_multiple(from, CACHE_LINE) 
}

/// Return the number of elements of `T` which will fit in some multiple
/// of the cache line size.
pub fn cache_aligned_len<T>() -> usize {
    math::lcm(CACHE_LINE, mem::size_of::<T>()) / mem::size_of::<T>()
}

// Get the bufb_offset and alloc_size for len 
fn alloc_info<T>(len: usize) -> (usize, usize) {
    let buf_size = size_of_alloc::<T>(len);
    let bufb_offset = next_cache_line(buf_size);
    // Overallocate to align to a cache line
    let alloc_size = bufb_offset + buf_size + CACHE_LINE;

    (bufb_offset, alloc_size)
}

#[test]
fn test_next_cache_line() {
    assert_eq!(next_cache_line(16), 64);
    assert_eq!(next_cache_line(33), 64);
    assert_eq!(next_cache_line(95), 128);
    assert_eq!(next_cache_line(128), 192);
}

