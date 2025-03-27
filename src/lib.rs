pub mod sync;

use std::{alloc::Layout, cell::{Cell, RefCell}, ops::{Deref, DerefMut}};

#[derive(Debug)]
pub struct FixBufferedAllocator<'buf> {
    buf: &'buf mut [u8],
    offset: usize,
}

impl<'buf> FixBufferedAllocator<'buf> {
    pub fn new(buf: &'buf mut [u8]) -> Self {
        Self { buf, offset: 0 }
    }

    const fn padding(&self, align: usize) -> usize {
        (self.offset.wrapping_neg()) & (align - 1)
    }

    pub fn alloc_raw(&mut self, layout: Layout) -> *mut u8 {
        let Some(aligned_offset) = self.offset.checked_add(self.padding(layout.align())) else {
            return std::ptr::null_mut();
        };

        let Some(total) = aligned_offset.checked_add(layout.size()) else {
            return std::ptr::null_mut();
        };

        if total > self.buf.len() { return std::ptr::null_mut(); }

        let ptr = unsafe {
            self.buf.as_mut_ptr().add(aligned_offset)
        };

        self.offset = total;

        ptr
    }

    pub fn alloc(&mut self, layout: Layout) -> Option<&'buf mut u8> {
        let ptr = self.alloc_raw(layout);
        if ptr.is_null() {
            None
        } else {    
            Some(unsafe { &mut *ptr })
        }
    }

    pub fn alloc_slice<T>(&mut self, length: usize) -> Option<&'buf mut [T]> {
        let size = std::mem::size_of::<T>().checked_mul(length)?;
        let align = std::mem::align_of::<T>();
        
        let layout = Layout::from_size_align(size, align).ok()?;
        let ptr = self.alloc_raw(layout) as *mut T;
        
        if ptr.is_null() {
            None
        } else {
            Some(unsafe { std::slice::from_raw_parts_mut(ptr, length) })
        }
    }

    pub fn create<T>(&mut self, value: T) -> Result<&'buf mut T, T> {
        let Some(res) = self.alloc(Layout::new::<T>()) else {
            return Err(value);
        };

        let res: &mut T = unsafe { std::mem::transmute(res) };

        *res = value;
        Ok(res)
    }
}

#[derive(Debug)]
pub struct RestartableFBA<'buf> {
    alloc: RefCell<FixBufferedAllocator<'buf>>,
    counter: Cell<usize>,
}

#[derive(Debug)]
pub struct AllocatedRef<'a, T: ?Sized> {
    reference: &'a mut T,
    allocator: &'a RestartableFBA<'a>,
}

impl<'a, T: ?Sized> Deref for AllocatedRef<'a, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.reference
    }
}

impl<'a, T: ?Sized> DerefMut for AllocatedRef<'a, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.reference
    }
}

impl<'a, T: ?Sized> Drop for AllocatedRef<'a, T> {
    fn drop(&mut self) {
        let counter = self.allocator.counter.get();
        assert!(counter >= 1);
        self.allocator.counter.set(counter - 1);
    }
}

impl<'buf> RestartableFBA<'buf> {
    pub fn new(buf: &'buf mut [u8]) -> Self {
        Self {
            alloc: RefCell::new(FixBufferedAllocator::new(buf)),
            counter: Cell::new(0)
        }
    }

    pub fn alloc<'alloc: 'buf>(&'alloc self, layout: Layout) -> Option<AllocatedRef<'buf, u8>> {
        let r = self.alloc.borrow_mut().alloc(layout)?;

        let counter = self.counter.get();
        self.counter.set(counter + 1);

        Some(AllocatedRef { reference: r, allocator: self })
    }    

    pub fn alloc_slice<'alloc: 'buf, T>(&'alloc self, length: usize) -> Option<AllocatedRef<'buf, [T]>> {
        let s = self.alloc.borrow_mut().alloc_slice::<T>(length)?;

        let counter = self.counter.get();
        self.counter.set(counter + 1);

        Some(AllocatedRef { reference: s, allocator: self })
    }

    pub fn create<'alloc: 'buf, T>(&'alloc self, value: T) -> Result<AllocatedRef<'buf, T>, T> {
        let r = self.alloc.borrow_mut().create(value)?;

        let counter = self.counter.get();
        self.counter.set(counter + 1);

        Ok(AllocatedRef { reference: r, allocator: self })
    }

    pub fn restart(&self) {
        self.try_restard().expect("Allocator can be restared only when there is no references to it's buffer")
    }

    pub fn new_buffer(&self, buf: &'buf mut [u8]) {
        self.try_new_buffer(buf).expect("New buffer of allocator can be setted only when there is no references to it's old buffer")
    }

    pub fn try_restard(&self) -> Option<()> {
        if self.counter.get() != 0 {
            None
        } else {
            self.alloc.borrow_mut().offset = 0;
            Some(())
        }
    }

    pub fn try_new_buffer(&self, buf: &'buf mut [u8]) -> Option<()> {
        if self.counter.get() != 0 {
            None
        } else {
            let mut alloc = self.alloc.borrow_mut();
            alloc.buf = buf;
            alloc.offset = 0;
            Some(())
        }
    }

    pub fn get_buf(&self) -> Option<&'buf mut [u8]> {
        if self.counter.get() != 0 {
            None
        } else {
            unsafe {
                Some(&mut (*self.alloc.as_ptr().cast::<FixBufferedAllocator<'buf>>()).buf)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_works() {
        let mut b = [0u8; 5];
        let mut a = FixBufferedAllocator::new(&mut b);
        dbg!(&a);
        let v1: &mut i32 = a.create(0x04030201).unwrap();
        let v2: &mut u8 = a.create(0xff).unwrap();
        
        assert_eq!(*v1, 0x04030201);
        assert_eq!(*v2, 0xff);
        dbg!(&a, v1, v2);
    }

    #[test]
    fn it_works1() {
        let mut b = [0u8; 12];
        let mut a = FixBufferedAllocator::new(&mut b);
        dbg!(&a);
        let v1: &i32 = a.create(0x04030201).unwrap();
        let v2: &u8 = a.create(0xff).unwrap();
        let v3: &u32 = a.create(0xfcfdfeff).unwrap();
        
        assert_eq!(*v1, 0x04030201);
        assert_eq!(*v2, 0xff);
        assert_eq!(*v3, 0xfcfdfeff);
        dbg!(&a, v1, v2, v3);
    }

    #[test]
    fn it_works2() {
        let mut b = [0u8; 12];
        let mut a = FixBufferedAllocator::new(&mut b);

        dbg!(&a);
        let s: &[u16] = a.create([1, 2, 3, 4]).unwrap();
        let v1: &u8 = a.create(0xff).unwrap();
        let v2: &u16 = a.create(0xaaaa).unwrap();
        let v3 = a.create(0xffu8);

        assert_eq!(*v1, 0xff);
        assert_eq!(*v2, 0xaaaa);
        assert_eq!(v3, Err(0xffu8));
        dbg!(&a, s, v1, v2, v3.unwrap_err());
    }

    #[test]
    fn it_works3() {
        let mut b = [0u8; 5];
        let mut a = FixBufferedAllocator::new(&mut b);

        dbg!(&a);
        let s: &mut [u8] = a.alloc_slice(5).unwrap();
        s.clone_from_slice("Hello".as_bytes());
        let s: &mut str = unsafe {std::mem::transmute(s)};
        dbg!(&a, s);
    }

    #[test]
    #[should_panic(expected = "Allocator can be restared only when there is no references to it's buffer")]
    fn restart_panics_with_active_references() {
        let mut b = [0u8; 2];
        let a = RestartableFBA::new(&mut b);

        let _v1 = a.create(5u8).unwrap();
        a.restart(); // This should panic
    }

    #[test]
    #[should_panic(expected = "New buffer of allocator can be setted only when there is no references to it's old buffer")]
    fn new_buffer_panics_with_active_references() {
        let mut b1 = [0u8; 2];
        let mut b2 = [0u8; 2];
        let a = RestartableFBA::new(&mut b1);

        let _v1 = a.create(5u8).unwrap();
        a.new_buffer(&mut b2); // This should panic
    }

    #[test]
    fn get_buf_test() {
        let mut b1 = [0u8; 1];
        let a = RestartableFBA::new(&mut b1);
        {
            let _ = a.create(255u8).unwrap();
        };

        let b = a.get_buf().unwrap();
        dbg!(&b);
        assert_eq!(&b, &[255]);
    }

    #[test]
    #[should_panic]
    fn get_buf_test_panic() {
        let mut b1 = [0u8; 1];
        let a = RestartableFBA::new(&mut b1);
        {
            let _ = a.create(255u8).unwrap();
        };

        dbg!(&a);
        let _b = a.get_buf().unwrap();

        let _ = a.create(1u8).unwrap();
    }
}
