use std::alloc::{alloc_zeroed, Layout};
use std::mem::{align_of, size_of};
use std::ptr::NonNull;
use std::slice;

const INLINED_ELEMENTS: usize = 3;
const FIRST_CHUNK_SIZE: usize = 8;

/// An Urolled Exponential Linked List.
pub struct Uell<T> {
    len: usize,
    first_chunk: Option<NonNull<Chunk<T>>>,
    last_chunk: Option<NonNull<Chunk<T>>>,
    last_elem_chunk: Option<NonNull<T>>,
    elems: [T; INLINED_ELEMENTS],
}

impl<T: Copy + Default> Uell<T> {
    pub fn new() -> Uell<T> {
        Uell {
            len: 0,
            first_chunk: None,
            last_chunk: None,
            last_elem_chunk: None,
            elems: [T::default(); INLINED_ELEMENTS],
        }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn push(&mut self, elem: T) {
        if self.len < INLINED_ELEMENTS {
            unsafe { *self.elems.get_unchecked_mut(self.len) = elem };
        } else {
            match self.last_elem_chunk {
                Some(ptr) if self.remaining_space() != 0 => {
                    let new_ptr = unsafe { ptr.as_ptr().offset(1) };
                    unsafe { new_ptr.write(elem) };
                    self.last_elem_chunk = NonNull::new(new_ptr);
                }
                _otherwise => {
                    let last_chunk = self.push_empty_chunk();
                    let new_ptr = unsafe { last_chunk.elems.get_unchecked_mut(0) };
                    *new_ptr = elem;
                    self.last_elem_chunk = NonNull::new(new_ptr);
                }
            }
        }

        self.len += 1;
    }

    /// Returns the number of elements that can be pushed
    /// before a new chunk is required.
    fn remaining_space(&self) -> usize {
        if self.len > INLINED_ELEMENTS {
            let mut len = self.len - INLINED_ELEMENTS;
            let mut next_chunk_size = FIRST_CHUNK_SIZE;
            while len > next_chunk_size {
                len -= next_chunk_size;
                next_chunk_size *= 2;
            }
            next_chunk_size - len
        } else {
            INLINED_ELEMENTS - self.len
        }
    }

    /// Returns the number of elements that the last chunk supports,
    /// `None` if there is no chunk allocated.
    fn last_chunk_size(&self) -> Option<usize> {
        if self.len > INLINED_ELEMENTS {
            let mut len = self.len - INLINED_ELEMENTS;
            let mut next_chunk_size = FIRST_CHUNK_SIZE;
            while len > next_chunk_size {
                len -= next_chunk_size;
                next_chunk_size *= 2;
            }
            Some(next_chunk_size)
        } else {
            None
        }
    }

    /// Allocates a new chunk that is twice the size of
    /// the last allocated chunk or 8 if there is no current chunk.
    fn push_empty_chunk(&mut self) -> &mut Chunk<T> {
        let size = self.last_chunk_size().unwrap_or(FIRST_CHUNK_SIZE);

        let last_chunk = Box::leak(Chunk::new(size));
        let mut last_chunk_ptr = NonNull::from(last_chunk);

        if self.first_chunk.is_none() {
            self.first_chunk = Some(last_chunk_ptr);
        }

        if let Some(mut last) = self.last_chunk {
            unsafe { last.as_mut().next = Some(last_chunk_ptr) };
        }

        self.last_chunk = Some(last_chunk_ptr);
        unsafe { last_chunk_ptr.as_mut() }
    }

    fn chunks_iter(&self) -> impl Iterator<Item = &[T]> {
        let mut next_chunk_size = FIRST_CHUNK_SIZE;
        let mut current_chunk = None;
        let mut len = self.len;
        std::iter::from_fn(move || {
            if len == 0 {
                None
            } else if current_chunk.is_none() {
                current_chunk = self.first_chunk.as_ref();
                let inlined_len = if current_chunk.is_none() {
                    len
                } else {
                    INLINED_ELEMENTS
                };
                let slice = &self.elems[..inlined_len];
                len -= inlined_len;
                Some(slice)
            } else {
                match current_chunk.take() {
                    Some(chunk) => {
                        let chunk = unsafe { chunk.as_ref() };
                        let size = if len < next_chunk_size {
                            len
                        } else {
                            next_chunk_size
                        };
                        let slice = &chunk.elems[..size];
                        current_chunk = chunk.next.as_ref();
                        len -= size;
                        next_chunk_size *= 2;
                        Some(slice)
                    }
                    None => None,
                }
            }
        })
    }
}

impl<T: Copy + Default> Default for Uell<T> {
    fn default() -> Uell<T> {
        Uell::new()
    }
}

impl<T> Drop for Uell<T> {
    fn drop(&mut self) {
        unsafe {
            let mut current_chunk = self.first_chunk.take().map(|p| Box::from_raw(p.as_ptr()));
            while let Some(mut chunk) = current_chunk.take() {
                current_chunk = chunk.next.take().map(|p| Box::from_raw(p.as_ptr()));
            }
        }
    }
}

// That's unsized
struct Chunk<T> {
    next: Option<NonNull<Chunk<T>>>,
    elems: [T],
}

impl<T: Copy> Chunk<T> {
    fn new(size: usize) -> Box<Chunk<T>> {
        let ptr = {
            let elems_size = size * size_of::<T>();
            let header_size = size_of::<Option<NonNull<Chunk<T>>>>();
            let size = header_size + elems_size;
            let align = align_of::<Option<Box<Chunk<T>>>>();
            let layout = unsafe { Layout::from_size_align_unchecked(size, align) };
            unsafe { alloc_zeroed(layout) }
        };

        // https://users.rust-lang.org/t/construct-fat-pointer-to-struct/29198/9
        fn fatten<T>(data: *mut u8, len: usize) -> *mut Chunk<T> {
            let slice = unsafe { slice::from_raw_parts(data as *mut (), len) };
            slice as *const [()] as *mut Chunk<T>
        }

        let chunk_ptr = fatten::<T>(ptr, size);
        unsafe { Box::from_raw(chunk_ptr) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    /// Push enough elements for them to be kept inlined (no allocated chunks).
    fn small_push() {
        let mut uell = Uell::new();

        for i in 0..INLINED_ELEMENTS {
            uell.push(i);
        }

        assert_eq!(uell.len(), INLINED_ELEMENTS);
    }

    #[test]
    /// Push enough elements to trigger a chunk allocation.
    fn bigger_push() {
        let mut uell = Uell::new();

        let count = INLINED_ELEMENTS + 10;
        for i in 0..count {
            uell.push(i);
        }

        assert_eq!(uell.len(), count);
    }

    #[test]
    /// Iterate over a small uell, and only the inlined elements.
    fn small_chunk_iter() {
        let mut uell = Uell::new();
        for i in 0..INLINED_ELEMENTS {
            uell.push(i);
        }

        let mut iter = uell.chunks_iter();
        assert_eq!(iter.next(), Some(&[0, 1, 2][..]));
        assert_eq!(iter.next(), None);
    }

    #[test]
    /// Iterate over a small uell, and only the inlined elements.
    fn bigger_chunk_iter() {
        let mut uell = Uell::new();
        for i in 0..(INLINED_ELEMENTS + 15) {
            uell.push(i);
        }

        let mut iter = uell.chunks_iter();
        assert_eq!(iter.next(), Some(&[0, 1, 2][..]));
        assert_eq!(iter.next(), Some(&[3, 4, 5, 6, 7, 8, 9, 10][..]));
        assert_eq!(iter.next(), Some(&[11, 12, 13, 14, 15, 16, 17][..]));
        assert_eq!(iter.next(), None);
    }
}
