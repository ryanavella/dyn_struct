//! This crate allows you to safely initialize Dynamically Sized Types (DST) using
//! only safe Rust.
//! 
//! ```ignore
//! #[repr(C)]
//! #[derive(DynStruct)]
//! struct MyDynamicType {
//!     pub awesome: bool,
//!     pub number: u32,
//!     pub dynamic: [u32],
//! }
//! 
//! // the `new` function is generated by the `DynStruct` macro.
//! let foo: Box<MyDynamicType> = MyDynamicType::new(true, 123, &[4, 5, 6, 7]);
//! assert_eq!(foo.awesome, true);
//! assert_eq!(foo.number, 123);
//! assert_eq!(&foo.dynamic, &[4, 5, 6, 7]);
//! ```
//! 
//! 
//! ## Why Dynamic Types?
//! 
//! In Rust, Dynamically Sized Types (DST) are everywhere. Slices (`[T]`) and trait
//! objects (`dyn Trait`) are the most common ones. However, it is also possible
//! to define your own! For example, this can be done by letting the last field in a
//! struct be a dynamically sized array (note the missing `&`):
//! 
//! ```ignore
//! struct MyDynamicType {
//!     awesome: bool,
//!     number: u32,
//!     dynamic: [u32],
//! }
//! ```
//! 
//! This tells the Rust compiler that contents of the `dynamic`-array is laid out in
//! memory right after the other fields. This can be very preferable in some cases,
//! since remove one level of indirection and increase cache-locality.
//! 
//! However, there's a catch! Just as with slices, the compiler does not know how
//! many elements are in `dynamic`. Thus, we need what is called a fat-pointer which
//! stores both a pointer to the actual data, but also the length of the array
//! itself. As of releasing this crate, the only safe way to construct a dynamic
//! type is if we know the size of the array at compile-time. However, for most use
//! cases, that is not possible. Therefore this crate uses some `unsafe` behind the
//! scenes to work around the limitations of the language, all wrapped up in a safe
//! interface.
//! 
//! 
//! ## The Derive Macro
//! 
//! The `DynStruct` macro can be applied to any `#[repr(C)]` struct that contains a
//! dynamically sized array as its last field. Fields only have a single constraint:
//! they have to implement `Copy`.
//! 
//! ### Example
//! 
//! ```ignore
//! #[repr(C)]
//! #[derive(DynStruct)]
//! struct MyDynamicType {
//!     pub awesome: bool,
//!     pub number: u32,
//!     pub dynamic: [u32],
//! }
//! ```
//! 
//! will produce a single `impl`-block with a `new` function:
//! 
//! ```ignore
//! impl MyDynamicType {
//!     pub fn new(awesome: bool, number: u32, dynamic: &[u32]) -> Box<MyDynamicType> {
//!         // ... implementation details ...
//!     }
//! }
//! ```
//! 
//! Due to the nature of dynamically sized types, the resulting value has to be
//! built on the heap. For safety reasons we currently only allow returning `Box`,
//! though in a future version we may also allow `Rc` and `Arc`. In the meantime it
//! is posible to use `Arc::from(MyDynamicType::new(...))`.


#[cfg(feature = "derive")]
pub use dyn_struct_derive::DynStruct;

#[repr(C)]
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DynStruct<T, D> {
    pub single: T,
    pub many: [D],
}

impl<T, D> DynStruct<T, D> {
    pub fn new(single: T, many: &[D]) -> Box<Self>
    where
        T: Copy,
        D: Copy,
    {
        use std::mem::{align_of, size_of};

        let total_size = size_of::<T>() + size_of::<D>() * many.len();

        if total_size == 0 {
            // Create a fat pointer to a slice of `many.len()` elements, then cast the slice into a
            // fat pointer to `Self`. This essentially creates the fat pointer to `Self` of
            // `many.len()` we need.
            let slice: Box<[()]> = Box::from(slice_with_len(many.len()));
            let ptr = Box::into_raw(slice) as *mut [()] as *mut Self;
            unsafe { Box::from_raw(ptr) }
        } else {
            let align = usize::max(align_of::<T>(), align_of::<D>());
            let layout = std::alloc::Layout::from_size_align(total_size, align).unwrap();

            unsafe {
                let raw = std::alloc::alloc(layout);
                if raw.is_null() {
                    std::alloc::handle_alloc_error(layout)
                }

                Self::single_ptr(raw).copy_from_nonoverlapping(&single as *const T, 1);
                Self::many_ptr(raw).copy_from_nonoverlapping(many.as_ptr(), many.len());

                let slice = std::slice::from_raw_parts_mut(raw as *mut (), many.len());
                let ptr = slice as *mut [()] as *mut Self;
                Box::from_raw(ptr)
            }
        }
    }

    fn single_ptr(raw: *mut u8) -> *mut T {
        raw as *mut T
    }

    fn many_ptr(raw: *mut u8) -> *mut D {
        unsafe {
            let naive = raw.add(std::mem::size_of::<T>());
            let align = std::mem::align_of::<D>();
            let ptr = naive.add(naive.align_offset(align));
            ptr as *mut D
        }
    }
}

impl<T> DynStruct<T, T> {
    /// Get a `DynStruct` as a view over a slice (this does not allocate).
    pub fn from_slice(values: &[T]) -> &Self {
        assert!(
            !values.is_empty(),
            "attempted to create `{}` without `single` value (`values.is_empty()`)",
            std::any::type_name::<Self>()
        );
        let slice = &values[..values.len() - 1];
        unsafe { &*(slice as *const [T] as *const Self) }
    }
}

fn slice_with_len(len: usize) -> &'static [()] {
    static ARBITRARY: [(); usize::MAX] = [(); usize::MAX];
    &ARBITRARY[..len]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mixed_types() {
        let mixed = DynStruct::new((true, 32u64), &[1, 2, 3, 4]);
        assert_eq!(mixed.single, (true, 32u64));
        assert_eq!(&mixed.many, &[1, 2, 3, 4]);
    }

    #[test]
    fn zero_sized_types() {
        let zero = DynStruct::new((), &[(), ()]);
        assert_eq!(zero.single, ());
        assert_eq!(&zero.many, &[(), ()]);
    }

    #[test]
    fn from_slice() {
        let same = DynStruct::<u32, u32>::from_slice(&[1, 2, 3]);
        assert_eq!(same.single, 1);
        assert_eq!(&same.many, &[2, 3]);
    }
}
