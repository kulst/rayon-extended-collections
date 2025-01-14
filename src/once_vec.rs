use std::{
    cell::UnsafeCell,
    collections::TryReserveError,
    iter::repeat_n,
    mem::{ManuallyDrop, MaybeUninit},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Once,
    },
};
#[derive(Debug)]
pub struct OnceVecError;
#[derive(Debug)]
pub struct OnceVec<T> {
    vec: Vec<UnsafeCell<MaybeUninit<T>>>,
    once: Vec<Once>,
    elements_written: AtomicUsize,
}

impl<T> OnceVec<T> {
    pub const fn new() -> Self {
        Self {
            vec: Vec::new(),
            once: Vec::new(),
            elements_written: AtomicUsize::new(0),
        }
    }

    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            vec: Vec::with_capacity(capacity),
            once: Vec::with_capacity(capacity),
            elements_written: AtomicUsize::new(0),
        }
    }

    pub fn with_uninit_len(len: usize) -> Self {
        Self {
            vec: repeat_n((), len)
                .map(|_| UnsafeCell::new(MaybeUninit::uninit()))
                .collect(),
            once: repeat_n((), len).map(|_| Once::new()).collect(),
            elements_written: AtomicUsize::new(0),
        }
    }

    pub fn capacity(&self) -> usize {
        self.vec.capacity()
    }

    pub fn reserve(&mut self, additional: usize) {
        self.vec.reserve(additional);
        self.once.reserve(additional);
    }

    pub fn reserve_exact(&mut self, additional: usize) {
        self.vec.reserve_exact(additional);
        self.once.reserve_exact(additional);
    }

    pub fn try_reserve(&mut self, additional: usize) -> Result<(), TryReserveError> {
        todo!();
    }

    pub fn try_reserve_exact(&mut self, additional: usize) -> Result<(), TryReserveError> {
        todo!();
    }

    pub fn shrink_to_fit(&mut self) {
        self.vec.shrink_to_fit();
        self.once.shrink_to_fit();
    }

    pub fn shrink_to(&mut self, min_capacity: usize) {
        self.vec.shrink_to_fit();
        self.once.shrink_to_fit();
    }

    // Still to decide if this should be implemented
    // pub fn into_boxed_slice(self) -> Box<[T], A>

    pub fn truncate(&mut self, len: usize) {
        if len >= self.vec.len() {
            return;
        }

        let elements_written = self.elements_written_until(len);
        self.vec.truncate(len);
        self.once.truncate(len);
        self.elements_written
            .store(elements_written, Ordering::Relaxed);
    }

    pub fn as_slice(&self) -> Result<&[T], OnceVecError> {
        if !self.is_fully_written() {
            Err(OnceVecError)
        } else {
            // SAFETY: casting `slice` returned from the inner vec to a `*const [T]` is safe since
            // it is made sure that the vec is fully written and therefore initialized
            // Also `MaybeUninit` is guaranteed to have the same layout as `T`.
            // The pointer is valid since it refers to memory owned by us which is a
            // reference and thus guaranteed to be valid for reads.
            let slice = unsafe {
                &*(self.vec.as_slice() as *const [UnsafeCell<MaybeUninit<T>>] as *const [T])
            };
            Ok(slice)
        }
    }

    pub fn as_mut_slice(&mut self) -> Result<&mut [T], OnceVecError> {
        if !self.is_fully_written() {
            Err(OnceVecError)
        } else {
            // SAFETY: similar to safety notes for `as_slice`, but we have a
            // mutable reference which is also guaranteed to be valid for writes.
            let slice = unsafe {
                &mut *(self.vec.as_mut_slice() as *mut [UnsafeCell<MaybeUninit<T>>] as *mut [T])
            };
            Ok(slice)
        }
    }

    pub fn as_vec(mut self) -> Result<Vec<T>, OnceVecError> {
        if !self.is_fully_written() {
            Err(OnceVecError)
        } else {
            let mut v = ManuallyDrop::new(self.vec);
            let ptr = v.as_mut_ptr() as *mut T;
            let length = v.len();
            let capacity = v.capacity();
            // SAFETY: We own `vec` so `ptr` results from an allocation with the global
            // allocator. `MaybeUninit` is guarantied to have the same layout as `T`.
            // Length and capacity are directly resulting from a safe vector so length
            // must be smaller than or equal to capacity.
            // We also check that the vector is fully written
            Ok(unsafe { Vec::from_raw_parts(ptr, length, capacity) })
        }
    }

    pub fn insert(&mut self, index: usize, element: T) {
        self.vec
            .insert(index, UnsafeCell::new(MaybeUninit::new(element)));
        let once = Once::new();
        let mut once_check = false;
        once.call_once(|| once_check = true);
        assert!(once_check);
        self.once.insert(index, once);
        self.elements_written.fetch_add(1, Ordering::Relaxed);
    }

    pub fn insert_uninit(&mut self, index: usize, element: MaybeUninit<T>) {
        self.vec.insert(index, UnsafeCell::new(element));
        self.once.insert(index, Once::new());
    }

    pub fn remove(&mut self, index: usize) -> Option<T> {
        let val = self.vec.remove(index);
        let once = self.once.remove(index);
        if once.is_completed() {
            self.elements_written.fetch_sub(1, Ordering::Relaxed);
            // SAFETY: We checked that the value was written before
            Some(unsafe { val.into_inner().assume_init() })
        } else {
            None
        }
    }

    pub fn remove_uninit(&mut self, index: usize) -> MaybeUninit<T> {
        let val = self.vec.remove(index);
        if self.once.remove(index).is_completed() {
            self.elements_written.fetch_sub(1, Ordering::Relaxed);
        }
        val.into_inner()
    }

    pub fn push(&mut self, value: T) {
        self.vec.push(UnsafeCell::new(MaybeUninit::new(value)));
        let once = Once::new();
        let mut once_check = false;
        once.call_once(|| once_check = true);
        assert!(once_check);
        self.once.push(once);
        self.elements_written.fetch_add(1, Ordering::Relaxed);
    }

    pub fn push_uninit(&mut self, value: MaybeUninit<T>) {
        self.vec.push(UnsafeCell::new(value));
        self.once.push(Once::new());
    }

    pub fn pop(&mut self) -> Option<T> {
        let val = self.vec.pop();
        let once = self.once.pop();
        if let Some(once) = once {
            if once.is_completed() {
                self.elements_written.fetch_sub(1, Ordering::Relaxed);
                // SAFETY: We checked that the value was written before
                Some(unsafe { val.unwrap().into_inner().assume_init() })
            } else {
                None
            }
        } else {
            None
        }
    }

    pub fn pop_uninit(&mut self) -> Option<MaybeUninit<T>> {
        let val = self.vec.pop();
        if let Some(once) = self.once.pop() {
            if once.is_completed() {
                self.elements_written.fetch_sub(1, Ordering::Relaxed);
            }
        }
        Some(val.unwrap().into_inner())
    }

    pub fn try_write(&self, index: usize, val: T) -> Result<(), OnceVecError> {
        let mut once_check = false;
        self.once[index].call_once(|| unsafe {
            let element = self.vec[index].get() as *mut T;
            *element = val;
            once_check = true;
        });
        if once_check == true {
            self.elements_written.fetch_add(1, Ordering::Relaxed);
            Ok(())
        } else {
            Err(OnceVecError)
        }
    }
}

impl<T> OnceVec<T> {
    fn elements_written_until(&self, until: usize) -> usize {
        self.once
            .iter()
            .take(until)
            .filter(|o| o.is_completed())
            .count()
    }

    fn elements_written(&self, until: usize) -> usize {
        self.elements_written_until(self.once.len())
    }

    fn is_fully_written(&self) -> bool {
        self.elements_written.load(Ordering::Relaxed) == self.once.len()
    }
}

#[test]
fn first_test() {
    let mut once_vec: OnceVec<f32> = OnceVec::with_uninit_len(6);
    let _ = once_vec.try_write(5, 0.35);
    assert!(once_vec.as_slice().is_err());
    let _ = once_vec.try_write(0, 0.30);
    let _ = once_vec.try_write(1, 0.31);
    let _ = once_vec.try_write(2, 0.32);
    let _ = once_vec.try_write(3, 0.33);
    let _ = once_vec.try_write(4, 0.34);
    assert_eq!(
        once_vec.as_slice().unwrap(),
        &[0.30, 0.31, 0.32, 0.33, 0.34, 0.35]
    );
    once_vec.as_mut_slice().unwrap()[5] = 0.40;
    assert_eq!(
        once_vec.as_slice().unwrap(),
        &[0.30, 0.31, 0.32, 0.33, 0.34, 0.40]
    );
    let vec = once_vec.as_vec().unwrap();
    assert_eq!(vec, vec![0.30, 0.31, 0.32, 0.33, 0.34, 0.40]);
}
