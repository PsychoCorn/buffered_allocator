use std::sync::{atomic::{AtomicUsize, Ordering}, Arc, Mutex};

use super::*;

#[derive(Debug)]
pub struct RestartableFBA<'buf> {
    alloc: Mutex<FixBufferedAllocator<'buf>>,
    counter: Arc<AtomicUsize>,
}

#[derive(Debug)]
pub struct AllocatedRef<'a, T: ?Sized> {
    reference: &'a mut T,
    counter: Arc<AtomicUsize>,
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
        assert!(self.counter.load(Ordering::Relaxed) >= 1);
        self.counter.fetch_sub(1, Ordering::Relaxed);
    }
}

impl<'buf> RestartableFBA<'buf> {
    pub fn new(buf: &'buf mut [u8]) -> Self {
        Self {
            alloc: Mutex::new(FixBufferedAllocator::new(buf)),
            counter: Arc::new(AtomicUsize::new(0))
        }
    }

    pub fn alloc<'alloc: 'buf>(&'alloc self, layout: Layout) -> Option<AllocatedRef<'buf, u8>> {
        let r = self.alloc.lock().unwrap().alloc(layout)?;

        self.counter.fetch_add(1, Ordering::Relaxed);

        Some(AllocatedRef { reference: r, counter: Arc::clone(&self.counter) })
    }    

    pub fn alloc_slice<'alloc: 'buf, T>(&'alloc self, length: usize) -> Option<AllocatedRef<'buf, [T]>> {
        let s = self.alloc.lock().unwrap().alloc_slice::<T>(length)?;

        self.counter.fetch_add(1, Ordering::Relaxed);

        Some(AllocatedRef { reference: s, counter: Arc::clone(&self.counter) })
    }

    pub fn create<'alloc: 'buf, T>(&'alloc self, value: T) -> Result<AllocatedRef<'buf, T>, T> {
        let r = self.alloc.lock().unwrap().create(value)?;

        self.counter.fetch_add(1, Ordering::Relaxed);

        Ok(AllocatedRef { reference: r, counter: Arc::clone(&self.counter) })
    }

    pub fn restart(&self) {
        self.try_restard().expect("Allocator can be restared only when there is no references to it's buffer")
    }

    pub fn new_buffer(&self, buf: &'buf mut [u8]) {
        self.try_new_buffer(buf).expect("New buffer of allocator can be setted only when there is no references to it's old buffer")
    }

    pub fn try_restard(&self) -> Option<()> {
        if self.counter.load(Ordering::Relaxed) != 0 {
            None
        } else {
            self.alloc.lock().unwrap().offset = 0;
            Some(())
        }
    }

    pub fn try_new_buffer(&self, buf: &'buf mut [u8]) -> Option<()> {
        if self.counter.load(Ordering::Relaxed) != 0 {
            None
        } else {
            let mut alloc = self.alloc.lock().unwrap();
            alloc.buf = buf;
            alloc.offset = 0;
            Some(())
        }
    }
}

#[cfg(test)]
mod tests {
    use std::thread;

    use super::*;

    #[test]
    fn it_works() {
        let mut b = [0u8; 5];
        let a = RestartableFBA::new(&mut b);
        dbg!(&a);
        let v1 = a.create(0x04030201).unwrap();
        let v2 = a.create(0xffu8).unwrap();
        
        assert_eq!(*v1, 0x04030201);
        assert_eq!(*v2, 0xff);
        dbg!(&a, v1, v2);
    }

    #[test]
    fn it_works1() {
        let mut buf = vec![0u8; 5];
        let a = RestartableFBA::new(&mut buf);

        thread::scope(|scope| {
            scope.spawn(|| {
                a.create(0x0201u16).unwrap();
                dbg!(&a);
            });

            scope.spawn(|| {
                a.create(0x0403u16).unwrap();
                dbg!(&a);
            });
        });

        a.create(5u8).unwrap();
        dbg!(&a);

        let mut buf2 = [0u8; 1];
        a.new_buffer(&mut buf2);
        let v = a.create(!0u8).unwrap();
        dbg!(&a);
        assert_eq!(*v, !0u8);
    }

    #[test]
    #[should_panic(expected = "Allocator can be restared only when there is no references to it's buffer")]
    fn it_works2() {
        let mut buf = vec![0u8; 5];
        let a = RestartableFBA::new(&mut buf);

        thread::scope(|scope| {
            scope.spawn(|| {
                a.create(0x0201u16)
            });

            scope.spawn(|| {
                a.create(0x0403u16)
            });
        });

        let v = a.create(5u8).unwrap();
        assert_eq!(*v, 5);
        dbg!(&a);

        a.restart();
    }

    #[test]
    fn it_works3() {
        let mut buf = [0u8; 16];
        let a = RestartableFBA::new(&mut buf);

        thread::scope(|scope| {
            scope.spawn(|| {
                let mut v = a.create(0u64).unwrap();
                let mut result = 0u64;
                for i in 0..1_000_000u64 {
                    result = result.wrapping_add(i);
                }
                *v = result;
                dbg!(&a);
            });

            scope.spawn(|| {
                let mut v = a.create(0u64).unwrap();
                let mut result = 0u64;
                for i in 1_000_000u64..2_000_000u64 {
                    result = result.wrapping_add(i);
                }
                *v = result;
                dbg!(&a);
            });
        });
        dbg!(&a);
        while let None = a.try_restard() {}
        
        dbg!(&a);
    }
}