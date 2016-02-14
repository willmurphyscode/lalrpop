use lr1::core::*;
use grammar::repr::*;
use petgraph::{EdgeDirection, Graph};
use petgraph::graph::{Edges, NodeIndex};
use std::fmt::{Debug, Formatter, Error};
use util::{Map, map};

#[cfg(test)] mod test;

/// Trace graphs are used to summarize how it is that we came to be in
/// a state where we can take some particular shift/reduce action; put
/// another way, how it is that we came to be in a state with some
/// particular LR(1) item.
///
/// The nodes in the graph are each labeled with a TraceGraphNode and
/// hence take one of two forms:
///
/// - TraceGraphNode::Item -- represents an LR0 item. These nodes are
///   used for the starting/end points in the graph only.  Basically a
///   complete trace stretches from the start item to some end item,
///   and all intermediate nodes are nonterminals.
/// - TraceGraphNode::Nonterminal -- if this graph is for a shift,
///   then these represent items where the cursor is at the beginning:
///   `X = (*) ...`. If the graph is for a reduce, they represent
///   items where a reduce is possible without shifting any more
///   terminals (though further reductions may be needed): `X =
///   ... (*) ...s` where `FIRST(...s)` includes `\epsilon`.
///
/// The edges in the graph are also important. They are labeled with
/// `SymbolSets` instances, meaning that each carries a (prefix,
/// cursor, and suffix) tuple. The label on an edge `A -> B` means
/// that transitioning from a state containing `A` to a state
/// containing `B` is possible if you:
///
/// - shift the symbols in `prefix`
/// - `B` will produce the symbol in `cursor`
/// - shift the symbols in `suffix` after `B` is popped
pub struct TraceGraph<'grammar> {
    // A -L-> B means:
    //
    //     Transition from a state containing A to a state containing
    //     B by (pushing|popping) the symbols L.
    //
    // If this trace graph represents a shift backtrace, then the
    // labels are symbols that are pushed. Otherwise they are labels
    // that are popped.
    graph: Graph<TraceGraphNode<'grammar>, SymbolSets<'grammar>>,
    indices: Map<TraceGraphNode<'grammar>, NodeIndex>,
}

#[derive(Copy, Clone, Debug, PartialOrd, Ord, PartialEq, Eq)]
pub enum TraceGraphNode<'grammar> {
    Nonterminal(NonterminalString),
    Item(LR0Item<'grammar>),
}

impl<'grammar> TraceGraph<'grammar> {
    pub fn new() -> TraceGraph<'grammar> {
        TraceGraph {
            graph: Graph::new(),
            indices: map(),
        }
    }

    pub fn add_node<T>(&mut self, node: T) -> NodeIndex
        where T: Into<TraceGraphNode<'grammar>>
    {
        let node = node.into();
        let graph = &mut self.graph;
        *self.indices.entry(node)
                     .or_insert_with(|| graph.add_node(node))
    }

    pub fn add_edge<F,T>(&mut self,
                         from: F,
                         to: T,
                         labels: SymbolSets<'grammar>)
        where F: Into<TraceGraphNode<'grammar>>,
              T: Into<TraceGraphNode<'grammar>>,
    {
        let from = self.add_node(from.into());
        let to = self.add_node(to.into());
        println!("add_edge({:?} -{:?}-> {:?})",
                 self.graph[from], labels, self.graph[to]);
        if !self.graph.edges_directed(from, EdgeDirection::Outgoing)
                      .any(|(t, &l)| t == to && l == labels)
        {
            self.graph.add_edge(from, to, labels);
        }
    }

    pub fn enumerate_paths_from<'graph>(&'graph self,
                                        lr0_item: LR0Item<'grammar>)
                                        -> PathEnumerator<'graph, 'grammar>
    {
        PathEnumerator::new(self, lr0_item)
    }
}

impl<'grammar> Into<TraceGraphNode<'grammar>> for NonterminalString {
    fn into(self) -> TraceGraphNode<'grammar> {
        TraceGraphNode::Nonterminal(self)
    }
}

impl<'grammar> Into<TraceGraphNode<'grammar>> for LR0Item<'grammar> {
    fn into(self) -> TraceGraphNode<'grammar> {
        TraceGraphNode::Item(self)
    }
}

impl<'grammar> Into<TraceGraphNode<'grammar>> for Item<'grammar> {
    fn into(self) -> TraceGraphNode<'grammar> {
        TraceGraphNode::Item(self.to_lr0())
    }
}

// This just exists to help with the `Debug` impl
struct TraceGraphEdge<'grammar> {
    from: TraceGraphNode<'grammar>,
    to: TraceGraphNode<'grammar>,
    label: (&'grammar [Symbol], Option<&'grammar Symbol>, &'grammar [Symbol]),
}

impl<'grammar> Debug for TraceGraphEdge<'grammar> {
    fn fmt(&self, fmt: &mut Formatter) -> Result<(), Error> {
        write!(fmt, "({:?} -{:?}-> {:?})", self.from, self.label, self.to)
    }
}

impl<'grammar> Debug for TraceGraph<'grammar> {
    fn fmt(&self, fmt: &mut Formatter) -> Result<(), Error> {
        let mut s = fmt.debug_list();
        for (&node, &index) in &self.indices {
            for (target, label) in
                self.graph.edges_directed(index, EdgeDirection::Outgoing)
            {
                s.entry(&TraceGraphEdge { from: node,
                                          to: self.graph[target],
                                          label: (label.prefix,
                                                  label.cursor,
                                                  label.suffix) });
            }
        }
        s.finish()
    }
}

///////////////////////////////////////////////////////////////////////////
// PathEnumerator
//
// The path enumerater walks a trace graph searching for paths that
// start at a given item and terminate at another item. If such a path
// is found, you can then find the complete list of symbols by calling
// `symbols_and_cursor` and also get access to the state.

pub struct PathEnumerator<'graph, 'grammar: 'graph> {
    graph: &'graph TraceGraph<'grammar>,
    stack: Vec<EnumeratorState<'graph, 'grammar>>,

    // The list of symbols for the current item.
    symbols: Vec<Symbol>,

    cursor: usize,
}

struct EnumeratorState<'graph, 'grammar: 'graph> {
    index: NodeIndex,
    symbol_sets: SymbolSets<'grammar>,
    edges: Edges<'graph, SymbolSets<'grammar>>,
}

impl<'graph, 'grammar> PathEnumerator<'graph, 'grammar> {
    fn new(graph: &'graph TraceGraph<'grammar>,
           lr0_item: LR0Item<'grammar>)
           -> Self {
        let start_state = graph.indices[&TraceGraphNode::Item(lr0_item)];
        let mut enumerator = PathEnumerator {
            graph: graph,
            stack: vec![],
            symbols: vec![],
            cursor: 0,
        };
        let edges = enumerator.incoming_edges(start_state);
        enumerator.stack.push(EnumeratorState {
            index: start_state,
            symbol_sets: SymbolSets::new(),
            edges: edges,
        });
        enumerator.find_next_trace();
        enumerator
    }

    /// Advance to the next example. Returns false if there are no more
    /// examples.
    pub fn advance(&mut self) -> bool {
        // If we have not yet exhausted all the examples, then the top
        // of the stack should be the last target item that we
        // found. Pop it off.
        match self.stack.pop() {
            Some(top_state) => {
                assert!(match self.graph.graph[top_state.index] {
                    TraceGraphNode::Item(_) => true,
                    TraceGraphNode::Nonterminal(_) => false,
                });

                self.find_next_trace()
            }
            None => {
                false
            }
        }
    }

    fn incoming_edges(&self, index: NodeIndex) -> Edges<'graph, SymbolSets<'grammar>> {
        self.graph.graph.edges_directed(index, EdgeDirection::Incoming)
    }

    /// This is the main operation, written in CPS style and hence it
    /// can seem a bit confusing. The idea is that `find_next_trace`
    /// is called when we are ready to consider the next child of
    /// whatever is on the top of the stack. It simply withdraws
    /// that next child (if any) and hands it to `push_next`.
    fn find_next_trace(&mut self) -> bool {
        println!("proceed()");
        if !self.stack.is_empty() {
            let next_edge = {
                let top_of_stack = self.stack.last_mut().unwrap();
                top_of_stack.edges.next()
            };
            self.push_next_child_if_any(next_edge)
        } else {
            false
        }
    }

    /// Invoked with the next child (if any) of the node on the top of
    /// the stack.
    ///
    /// If `next` is `Some`, we simply call `push_next_child`.
    ///
    /// If `next` is `None`, then the node on the top of
    /// the stack *has* no next child, and so it is popped, and then
    /// we call `find_next_trace` again to start with the next child
    /// of the new top of the stack.
    fn push_next_child_if_any(&mut self,
                              next: Option<(NodeIndex, &'graph SymbolSets<'grammar>)>)
                              -> bool {
        if let Some((index, &symbol_sets)) = next {
            self.push_next_child(index, symbol_sets)
        } else {
            self.stack.pop();
            self.find_next_trace()
        }
    }

    /// Push the next child of the top of the stack onto the stack,
    /// making the child the new top.
    ///
    /// If the child is an `Item` node, we have found the next trace,
    /// and hence our search terminates. We push the symbols from this
    /// item node into the symbols vector and then call `found_trace`
    /// (which will ultimately return `true`).
    ///
    /// Otherwise, we check whether this new node would cause a cycle.
    /// If so, we do *not* push it, and instead just call
    /// `find_next_trace` again to proceed to the next child of the
    /// current top.
    ///
    /// Finally, if the new node would NOT cause a cycle, then we can
    /// push it onto the stack so that it becomes the new top, and
    /// call `find_next_trace` to start searching its children.
    fn push_next_child(&mut self,
                       index: NodeIndex,
                       symbol_sets: SymbolSets<'grammar>)
                       -> bool {
        println!("push(index={:?}, symbol_sets={:?}",
                 self.graph.graph[index], symbol_sets);

        match self.graph.graph[index] {
            TraceGraphNode::Item(_) => {
                // If we reached an item like
                //
                //     X = ...p (*) ...s
                //
                // then we are done, but we still need to push on the
                // symbols `...p`.
                let edges = self.incoming_edges(index);
                self.stack.push(EnumeratorState {
                    index: index,
                    symbol_sets: symbol_sets,
                    edges: edges,
                });
                self.found_trace()
            }
            TraceGraphNode::Nonterminal(_) => {
                // If this node already appears on the stack, do not
                // visit its children.
                if !self.stack.iter().any(|state| state.index == index) {
                    let edges = self.incoming_edges(index);
                    self.stack.push(EnumeratorState {
                        index: index,
                        symbol_sets: symbol_sets,
                        edges: edges,
                    });
                }
                self.find_next_trace()
            }
        }
    }

    // Assemble the `symbols` vector and `cursor`
    fn found_trace(&mut self)
                   -> bool {
        println!("found_trace()");

        self.symbols.truncate(0);

        self.symbols.extend(
            self.stack.iter()
                      .rev()
                      .flat_map(|s| s.symbol_sets.prefix));

        self.cursor = self.symbols.len();

        self.symbols.extend(
            self.stack[1].symbol_sets.cursor);

        self.symbols.extend(
            self.stack.iter()
                      .flat_map(|s| s.symbol_sets.suffix));

        println!("found_trace: symbols={:?} cursor={:?}",
                 self.symbols, self.cursor);
        true
    }

    /// Return the symbols of the current trace, or None if there is
    /// no current trace.
    pub fn symbols_and_cursor(&self) -> Option<(&[Symbol], usize)> {
        if self.stack.is_empty() {
            None
        } else {
            Some((&self.symbols[..], self.cursor))
        }
    }

    fn stack(&self) -> &[EnumeratorState<'graph, 'grammar>] {
        &self.stack
    }
}

impl<'graph, 'grammar> Iterator for PathEnumerator<'graph, 'grammar> {
    type Item = (Vec<Symbol>, usize);

    fn next(&mut self) -> Option<Self::Item> {
        let this =
            self.symbols_and_cursor()
                .map(|(symbols, cursor)| (symbols.to_vec(), cursor));
        self.advance();
        this
    }
}

///////////////////////////////////////////////////////////////////////////
// ExampleEnumerator
//
// Wraps a path enumerater and builds examples.
//
//pub struct ExampleEnumerator<'graph, 'grammar: 'graph> {
//    paths: PathEnumerator<'graph, 'grammar>,
//}
//
//impl<'graph, 'grammar> Iterator for PathEnumerator<'graph, 'grammar> {
//    type Item = (Vec<Symbol>, usize);
//
//    fn next(&mut self) -> Option<Self::Item> {
//        let this =
//            self.paths
//                .symbols_and_cursor()
//                .map(|(symbols, cursor)| {
//                    // The bottom of the path enumerator stack (index
//                    // 0) is the starting item, but all the other
//                    // entries are nonterminal intermediate nodes that
//                    // represent reductions. Convert those into the
//                    // reductions vector.
//                    let reductions =
//                        self.paths
//                            .stack()
//                            .iter()
//                            .skip(1)
//                            .map(|stack_elem| {
//                                Reduction
//                            });
//
//                    Example {
//                        symbols: symbols.to_vec(),
//                        cursor: cursor,
//                        reductions:
//                    }
//                });
//
//        self.paths.advance();
//        this
//    }
//}
