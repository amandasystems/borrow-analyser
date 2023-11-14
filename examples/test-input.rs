use std::cell::RefCell;

pub fn position_dependent_outlives(x: &mut i32, cond: bool) -> &mut i32 {
    let y = &mut *x;
    if cond {
        return y;
    } else {
        *x = 0;
        return x;
    }
}

struct Node {
    next: Option<RefCell<Box<Node>>>,
}

impl Node {
    fn print_nodes(head: &Node) {
        let mut next = Some(head);
        let next2;
        if let Some(n) = next {
            match &n.next {
                Some(n2) => {
                    next2 = n2.borrow();
                    next = Some(next2.as_ref());
                }
                _ => {}
            }
        }
    }
}

pub struct Collection {
    buckets: Vec<i32>,
}

impl Collection {
    fn get_bucket_index(&self) -> usize {
        18
    }

    fn errors(&mut self) {
        // Error: overlapping loans!
        let v = self.buckets.get_mut(self.get_bucket_index());
        todo!()
    }
}

fn next<'buf>(buffer: &'buf mut String) -> &'buf str {
    loop {
        let event = parse(buffer);

        if true {
            return event;
        }
    }
}

fn parse<'buf>(_buffer: &'buf mut String) -> &'buf str {
    unimplemented!()
}

pub fn main() {
    println!("Hello!");
}

fn main2() {
    let a = 5;
    let b = |_| &a;
    bad(&b);
}

fn bad<F: Fn(&i32) -> &i32>(_: F) {}
