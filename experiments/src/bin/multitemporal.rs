#[macro_use]
extern crate abomonation_derive;
extern crate abomonation;

extern crate rand;
extern crate timely;
extern crate differential_dataflow;

use rand::{Rng, SeedableRng, StdRng};

use timely::dataflow::ProbeHandle;
// use timely::progress::timestamp::RootTimestamp;

use timely::dataflow::operators::unordered_input::UnorderedInput;

use differential_dataflow::AsCollection;
use differential_dataflow::operators::*;

use pair::Pair;

fn main() {

    let nodes: usize = std::env::args().nth(1).unwrap().parse().unwrap();
    let edges: usize = std::env::args().nth(2).unwrap().parse().unwrap();
    let batch: usize = std::env::args().nth(3).unwrap().parse().unwrap();
    let rounds: usize = std::env::args().nth(4).unwrap().parse().unwrap();

    timely::execute_from_args(std::env::args().skip(5), move |worker| {

        let index = worker.index();
        let peers = worker.peers();
        let mut probe = ProbeHandle::new();

        let (mut root_input, root_cap, mut edge_input, mut edge_cap) =
        worker.dataflow(|scope| {

            let ((root_input, root_cap), roots) = scope.new_unordered_input();
            let ((edge_input, edge_cap), edges) = scope.new_unordered_input();

            let roots = roots.as_collection();
            let edges = edges.as_collection()
                             // .inspect(|x| println!("edge: {:?}", x))
                             ;

            roots.iterate(|inner| {

                let edges = edges.enter(&inner.scope());
                let roots = roots.enter(&inner.scope());

                edges
                    .semijoin(&inner)
                    .map(|(_s,d)| d)
                    .concat(&roots)
                    .distinct()
            })
            .consolidate()
            .inspect(|x| println!("edge: {:?}", x))
            .map(|_| ())
            .consolidate()
            // .inspect(|x| println!("{:?}\tchanges: {:?}", x.1, x.2))
            .probe_with(&mut probe);

            (root_input, root_cap, edge_input, edge_cap)
        });

        let seed: &[_] = &[1, 2, 3, index];
        let mut rng1: StdRng = SeedableRng::from_seed(seed);    // rng for edge additions
        let mut rng2: StdRng = SeedableRng::from_seed(seed);    // rng for edge deletions

        let worker_edges = edges / peers + if index < edges % peers { 1 } else { 0 };
        let worker_batch = batch / peers + if index < batch % peers { 1 } else { 0 };

        // Times: (revision, event_time)

        // load initial root.
        root_input
            .session(root_cap)
            .give((0, Pair::new(0, 0), 1));

        // load initial edges
        edge_input
            .session(edge_cap.clone())
            .give_iterator((0 .. worker_edges).map(|_|
                ((rng1.gen_range(0, nodes), rng1.gen_range(0, nodes)),
                    Pair::new(0, 0), 1)
            ));

        let edge_cap_next = edge_cap.delayed(&Pair::new(1, 0));

        // Caps = { (1,0) , (0,1) }

        edge_cap.downgrade(&Pair::new(0, 1));
        while probe.less_than(edge_cap.time()) {
            worker.step();
        }

        println!("Initial computation complete");

        for round in 1 .. rounds {

            edge_input
                .session(edge_cap.clone())
                .give_iterator((0 .. worker_batch).flat_map(|_| {
                    let insert = ((rng1.gen_range(0, nodes), rng1.gen_range(0, nodes)), Pair::new(0, round), 1);
                    let remove = ((rng2.gen_range(0, nodes), rng2.gen_range(0, nodes)), Pair::new(0, round),-1);
                    Some(insert).into_iter().chain(Some(remove).into_iter())
                }));

            edge_cap.downgrade(&Pair::new(0, round+1));
            while probe.less_than(edge_cap.time()) {
                worker.step();
            }

            // Caps = { (1,0), (0,round+1) }
            println!("Initial round {} complete", round);
        }

        // Caps = { (1,0) }
        edge_cap = edge_cap_next;

        edge_input
            .session(edge_cap.clone())
            .give_iterator((0 .. worker_batch).map(|_| {
                ((rng1.gen_range(0, nodes), rng1.gen_range(0, nodes)), Pair::new(1, 0), 1)
            }));

        // Caps = { (2,0) }
        edge_cap.downgrade(&Pair::new(2, 0));
        while probe.less_than(&Pair::new(1, rounds+1)) {
            worker.step();
        }

        edge_input
            .session(edge_cap.clone())
            .give_iterator((0 .. worker_batch).map(|_| {
                ((rng1.gen_range(0, nodes), rng1.gen_range(0, nodes)), Pair::new(2, 3), 1)
            }));

        // Caps = { (3,0) }
        edge_cap.downgrade(&Pair::new(3, 0));
        while probe.less_than(&Pair::new(2, rounds+1)) {
            worker.step();
        }

        edge_input
            .session(edge_cap.clone())
            .give_iterator((0 .. worker_batch).map(|_| {
                ((rng1.gen_range(0, nodes), rng1.gen_range(0, nodes)), Pair::new(3, 1), 1)
            }));

        // Caps = { (4,0) }
        edge_cap.downgrade(&Pair::new(4, 0));
        while probe.less_than(&Pair::new(3, rounds+1)) {
            worker.step();
        }


    }).unwrap();
}

/// This module contains a definition of a new timestamp time, a "pair" or product.
///
/// This is a minimal self-contained implementation, in that it doesn't borrow anything
/// from the rest of the library other than the traits it needs to implement. With this
/// type and its implementations, you can use it as a timestamp type.
mod pair {

    /// A pair of timestamps, partially ordered by the product order.
    #[derive(Debug, Hash, Default, Clone, Eq, PartialEq, Ord, PartialOrd, Abomonation)]
    pub struct Pair<S, T> {
        pub first: S,
        pub second: T,
    }

    impl<S, T> Pair<S, T> {
        /// Create a new pair.
        pub fn new(first: S, second: T) -> Self {
            Pair { first, second }
        }
    }

    // Implement timely dataflow's `PartialOrder` trait.
    use timely::order::PartialOrder;
    impl<S: PartialOrder, T: PartialOrder> PartialOrder for Pair<S, T> {
        fn less_equal(&self, other: &Self) -> bool {
            self.first.less_equal(&other.first) && self.second.less_equal(&other.second)
        }
    }

    use timely::progress::timestamp::Refines;
    impl<S: Timestamp, T: Timestamp> Refines<()> for Pair<S, T> {
        fn to_inner(outer: ()) -> Self { Default::default() }
        fn to_outer(self) -> () { () }
        fn summarize(summary: <Self>::Summary) -> () { () }
    }

    // Implement timely dataflow's `PathSummary` trait.
    // This is preparation for the `Timestamp` implementation below.
    use timely::progress::PathSummary;
    // impl<S: Timestamp, T: Timestamp> PathSummary<Pair<S,T>> for Pair<<S as Timestamp>::Summary, <T as Timestamp>::Summary> {
    //     fn results_in(&self, timestamp: &Pair<S, T>) -> Option<Pair<S,T>> {
    //         if let Some(s) = self.first.results_in(&timestamp.first) {
    //             if let Some(t) = self.second.results_in(&timestamp.second) {
    //                 Some(Pair::new(s,t))
    //             }
    //             else { None }
    //         }
    //         else { None }
    //     }
    //     fn followed_by(&self, other: &Self) -> Option<Self> {
    //         if let Some(s) = self.first.followed_by(&other.first) {
    //             if let Some(t) = self.second.followed_by(&other.second) {
    //                 Some(Pair::new(s,t))
    //             }
    //             else { None }
    //         }
    //         else { None }
    //     }
    // }

    impl<S: Timestamp, T: Timestamp> PathSummary<Pair<S,T>> for () {
        fn results_in(&self, timestamp: &Pair<S, T>) -> Option<Pair<S,T>> {
            Some(timestamp.clone())
        }
        fn followed_by(&self, other: &Self) -> Option<Self> {
            Some(other.clone())
        }
    }

    // Implement timely dataflow's `Timestamp` trait.
    use timely::progress::Timestamp;
    impl<S: Timestamp, T: Timestamp> Timestamp for Pair<S, T> {
        type Summary = ();//Pair<<S as Timestamp>::Summary, <T as Timestamp>::Summary>;
    }

    // Implement differential dataflow's `Lattice` trait.
    // This extends the `PartialOrder` implementation with additional structure.
    use differential_dataflow::lattice::Lattice;
    impl<S: Lattice, T: Lattice> Lattice for Pair<S, T> {
        fn minimum() -> Self { Pair { first: S::minimum(), second: T::minimum() }}
        fn maximum() -> Self { Pair { first: S::maximum(), second: T::maximum() }}
        fn join(&self, other: &Self) -> Self {
            Pair {
                first: self.first.join(&other.first),
                second: self.second.join(&other.second),
            }
        }
        fn meet(&self, other: &Self) -> Self {
            Pair {
                first: self.first.meet(&other.first),
                second: self.second.meet(&other.second),
            }
        }
    }
}