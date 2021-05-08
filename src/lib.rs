use std::mem::{size_of, transmute};
use std::ptr::NonNull;

const INLINED_ELEMENTS: usize = 3;
const FIRST_CHUNK_SIZE: usize = 8;

/// An Urolled Exponential Linked List.
pub struct Uell<T> {
    len: usize,
    first_chunk: Option<Box<Chunk<T>>>,
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

    pub fn push(&mut self, elem: T) {
        if self.len < self.elems.len() {
            unsafe { *self.elems.get_unchecked_mut(self.len) = elem };
        } else {
            match self.last_elem_chunk {
                Some(ptr) if self.remaining_space() != 0 => {
                    let new_ptr = unsafe { ptr.as_ptr().offset(1) };
                    unsafe { *new_ptr = elem };
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

    /// Allocate a new chunk that is the twice the size of
    /// the last allocated chunk or 8 if there is no current chunk.
    fn push_empty_chunk(&mut self) -> &mut Chunk<T> {
        let size = self.last_chunk_size().unwrap_or(FIRST_CHUNK_SIZE);
        let mut last_chunk = Chunk::new(size);
        let ref_last_chunk = &mut *last_chunk;
        let last_chunk_ptr = NonNull::new(ref_last_chunk);
        if let Some(mut last) = self.last_chunk {
            unsafe { last.as_mut().next = last_chunk_ptr };
        }
        self.last_chunk = last_chunk_ptr;
        // TODO remove this unwrap
        unsafe { &mut *self.last_chunk.unwrap().as_ptr() }
    }
}

// That's unsized
struct Chunk<T> {
    next: Option<NonNull<Chunk<T>>>,
    elems: [T],
}

impl<T: Copy + Default> Chunk<T> {
    fn new(size: usize) -> Box<Chunk<T>> {
        let elems_size = size_of::<T>();
        let header_size = size_of::<Option<NonNull<Chunk<T>>>>();
        let raw_box: Box<[u8]> = vec![0u8; header_size + elems_size].into_boxed_slice();
        let mut chunk: Box<Chunk<T>> = unsafe { transmute(raw_box) };

        chunk.elems[..size]
            .iter_mut()
            .for_each(|e| *e = T::default());

        chunk
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
}
