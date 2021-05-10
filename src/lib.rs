use std::alloc::{alloc_zeroed, Layout};
use std::mem::{align_of, size_of};
use std::ptr::NonNull;
use std::slice;

const INLINED_ELEMENTS: usize = 3;
const FIRST_CHUNK_SIZE: usize = 8;

/// An Unrolled Exponential Linked List.
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

    pub fn into_iter(self) -> IntoIter<T> {
        IntoIter::new(self)
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

    /// Returns the capacity of the last allocated chunk
    /// or `None` if there is no chunk allocated.
    fn last_chunk_size(&self) -> Option<usize> {
        match self.last_chunk {
            Some(chunk) => unsafe { Some(chunk.as_ref().capacity()) },
            None => None,
        }
    }

    /// Allocates a new chunk that is twice the size of
    /// the last allocated chunk or 8 if there is no current chunk.
    fn push_empty_chunk(&mut self) -> &mut Chunk<T> {
        let size = self.last_chunk_size().map(|size| size * 2).unwrap_or(FIRST_CHUNK_SIZE);

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

struct IntoChunkIter<T> {
    chunk: Option<Box<Chunk<T>>>,
}

impl<T> IntoChunkIter<T> {
    fn new(chunk: Option<NonNull<Chunk<T>>>) -> IntoChunkIter<T> {
        IntoChunkIter { chunk: chunk.map(|p| unsafe { Box::from_raw(p.as_ptr()) }) }
    }
}

impl<T> Iterator for IntoChunkIter<T> {
    type Item = Box<Chunk<T>>;

    fn next(&mut self) -> Option<Self::Item> {
        match self.chunk.take() {
            Some(mut chunk) => {
                let next_chunk = chunk.next.take().map(|p| unsafe { Box::from_raw(p.as_ptr()) });
                self.chunk = next_chunk;
                Some(chunk)
            }
            None => None,
        }
    }
}

pub struct IntoIter<T> {
    inner: InnerIntoIter<T>,
}

enum InnerIntoIter<T> {
    Inline {
        elems: [T; INLINED_ELEMENTS],
        inline_offset: usize,
        chunks: Option<NonNull<Chunk<T>>>,
        len: usize,
    },
    Chunks {
        current_chunk: Option<Box<Chunk<T>>>,
        chunk_offset: usize,
        chunk_iter: IntoChunkIter<T>,
        remaining_len: usize,
    },
}

impl<T: Copy> IntoIter<T> {
    fn new(mut uell: Uell<T>) -> IntoIter<T> {
        IntoIter {
            inner: InnerIntoIter::Inline {
                elems: uell.elems,
                inline_offset: 0,
                chunks: uell.first_chunk.take(),
                len: uell.len,
            },
        }
    }

    fn new_from_chunks(len: usize, chunks: Option<NonNull<Chunk<T>>>) -> IntoIter<T> {
        let mut chunk_iter = IntoChunkIter::new(chunks);
        IntoIter {
            inner: InnerIntoIter::Chunks {
                current_chunk: chunk_iter.next(),
                chunk_offset: 0,
                chunk_iter,
                remaining_len: len - INLINED_ELEMENTS,
            },
        }
    }
}

impl<T: Copy> Iterator for IntoIter<T> {
    type Item = T;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            match &mut self.inner {
                InnerIntoIter::Inline { elems, inline_offset, chunks, len } => {
                    if *inline_offset == elems.len() {
                        *self = IntoIter::new_from_chunks(*len, chunks.take());
                    } else if *inline_offset < *len {
                        let elem = elems[*inline_offset];
                        *inline_offset += 1;
                        return Some(elem);
                    } else {
                        return None;
                    }
                }
                InnerIntoIter::Chunks {
                    current_chunk,
                    chunk_iter,
                    chunk_offset,
                    remaining_len,
                } => match current_chunk {
                    Some(chunk) => {
                        if *remaining_len == 0 {
                            return None;
                        } else if *chunk_offset == chunk.capacity() {
                            *current_chunk = chunk_iter.next();
                            *chunk_offset = 0;
                        } else {
                            let elem = chunk.elems[*chunk_offset];
                            *chunk_offset += 1;
                            *remaining_len -= 1;
                            return Some(elem);
                        }
                    }
                    None => return None,
                },
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

    fn capacity(&self) -> usize {
        self.elems.len()
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
    /// Push a small amount of elements and therefore
    /// only iter on the inlined elements.
    fn small_push_into_iter() {
        let mut uell = Uell::new();

        for i in 0..(INLINED_ELEMENTS - 1) {
            uell.push(i);
        }

        let mut iter = uell.into_iter();
        assert_eq!(iter.next(), Some(0));
        assert_eq!(iter.next(), Some(1));
        assert_eq!(iter.next(), None);
    }

    #[test]
    /// Push a big amount of elements and therefore
    /// iter on the inlined and then the chunked elements.
    fn bigger_push_into_iter() {
        let mut uell = Uell::new();

        for i in 0..(INLINED_ELEMENTS + 100) {
            uell.push(i);
        }

        assert!(uell.into_iter().eq(0..INLINED_ELEMENTS + 100));
    }

    #[test]
    /// Push a big amount of elements and iter on the allocated chunks.
    fn chunk_iter() {
        let mut uell = Uell::new();

        let len = INLINED_ELEMENTS + 100;
        for i in 0..len {
            uell.push(i);
        }

        let first_chunk = uell.first_chunk.take();
        let iter = IntoChunkIter::new(first_chunk);
        assert_eq!(iter.count(), 4);
    }
}
