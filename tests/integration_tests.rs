#[test]
fn test_rendezvous() {
    use rendezvous_swap::Rendezvous;
    use std::thread;

    let (mut my_rendezvous, mut their_rendezvous) = Rendezvous::new();
    thread::spawn(move || {
        for i in 1..5 {
            println!("{i}");
            their_rendezvous.wait();
        }
    });
    for i in 1..5 {
        println!("{i}");
        my_rendezvous.wait();
    }
}
#[test]
fn test_rendezvous_data() {
    use rendezvous_swap::RendezvousData;
    use std::thread;

    let (mut my_rendezvous, mut their_rendezvous) = RendezvousData::new(0, 0);
    let handle = thread::spawn(move || {
        let borrow = their_rendezvous.swap();
        *borrow = 3;

        let borrow = their_rendezvous.swap();
        assert_eq!(7, *borrow);
    });
    let borrow = my_rendezvous.swap();
    *borrow = 7;

    let borrowed_data = my_rendezvous.swap();
    assert_eq!(3, *borrowed_data);

    handle.join().unwrap();
}
