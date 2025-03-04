use self::atoms::{AllFacts, AtomMaps, Loan, Origin, Output, Path, SubPoint};
use crate::context::{AnalysisCtxt, PermissionSet};
use crate::dataflow::DataflowConstraints;
use crate::labeled_ty::{LabeledTy, LabeledTyCtxt};
use crate::pointer_id::PointerTableMut;
use crate::util::{describe_rvalue, RvalueDesc};
use rustc_middle::mir::{Body, BorrowKind, Local, LocalKind, Place, StatementKind, START_BLOCK};
use rustc_middle::ty::{List, TyKind};
use std::collections::HashMap;
use std::hash::Hash;

mod atoms;
mod def_use;
mod dump;
mod type_check;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash, Default)]
pub struct Label {
    pub origin: Option<Origin>,
    pub perm: PermissionSet,
}

pub type LTy<'tcx> = LabeledTy<'tcx, Label>;
pub type LTyCtxt<'tcx> = LabeledTyCtxt<'tcx, Label>;

pub fn borrowck_mir<'tcx>(
    acx: &AnalysisCtxt<'_, 'tcx>,
    dataflow: &DataflowConstraints,
    hypothesis: &mut PointerTableMut<PermissionSet>,
    name: &str,
    mir: &Body<'tcx>,
) {
    let mut i = 0;
    loop {
        eprintln!("run polonius");
        let (facts, maps, output) = run_polonius(acx, hypothesis, name, mir);
        eprintln!(
            "polonius: iteration {}: {} errors, {} move_errors",
            i,
            output.errors.len(),
            output.move_errors.len(),
        );
        i += 1;

        if output.errors.is_empty() {
            break;
        }
        if i >= 20 {
            panic!()
        }

        let mut changed = false;
        for loans in output.errors.values() {
            for &loan in loans {
                let issued_point = facts
                    .loan_issued_at
                    .iter()
                    .find(|&&(_, l, _)| l == loan)
                    .map(|&(_, _, point)| point)
                    .unwrap_or_else(|| panic!("loan {:?} was never issued?", loan));
                let issued_loc = maps.get_point_location(issued_point);
                let stmt = mir.stmt_at(issued_loc).left().unwrap_or_else(|| {
                    panic!(
                        "loan {:?} was issued by a terminator (at {:?})?",
                        loan, issued_loc
                    );
                });
                let ptr = match stmt.kind {
                    StatementKind::Assign(ref x) => match describe_rvalue(&x.1) {
                        Some(RvalueDesc::Project { base, proj: _ }) => acx
                            .ptr_of(base)
                            .unwrap_or_else(|| panic!("missing pointer ID for {:?}", base)),
                        Some(RvalueDesc::AddrOfLocal { local, proj: _ }) => {
                            acx.addr_of_local[local]
                        }
                        None => panic!("loan {:?} was issued by unknown rvalue {:?}?", loan, x.1),
                    },
                    _ => panic!("loan {:?} was issued by non-assign stmt {:?}?", loan, stmt),
                };
                eprintln!("want to drop UNIQUE from pointer {:?}", ptr);

                if hypothesis[ptr].contains(PermissionSet::UNIQUE) {
                    hypothesis[ptr].remove(PermissionSet::UNIQUE);
                    changed = true;
                }
            }
        }

        eprintln!("propagate");
        changed |= dataflow.propagate(hypothesis);
        eprintln!("done propagating");

        if !changed {
            eprintln!(
                "{} unresolved borrowck errors in function {:?} (after {} iterations)",
                output.errors.len(),
                name,
                i,
            );
            break;
        }
    }
}

fn run_polonius<'tcx>(
    acx: &AnalysisCtxt<'_, 'tcx>,
    hypothesis: &PointerTableMut<PermissionSet>,
    name: &str,
    mir: &Body<'tcx>,
) -> (AllFacts, AtomMaps<'tcx>, Output) {
    let tcx = acx.tcx();
    let mut facts = AllFacts::default();
    let mut maps = AtomMaps::default();

    // Start the origin counter at 3.  This has no effect on the semantics, but makes for easier
    // diffs between our facts and the facts generated by rustc.
    for _ in 0..3 {
        let _ = maps.origin();
    }

    //pretty::write_mir_fn(tcx, mir, &mut |_, _| Ok(()), &mut std::io::stdout()).unwrap();

    // Populate `cfg_edge`
    for (bb, bb_data) in mir.basic_blocks().iter_enumerated() {
        eprintln!("{:?}:", bb);

        for idx in 0..bb_data.statements.len() {
            eprintln!("  {}: {:?}", idx, bb_data.statements[idx]);
            let start = maps.point(bb, idx, SubPoint::Start);
            let mid = maps.point(bb, idx, SubPoint::Mid);
            let next_start = maps.point(bb, idx + 1, SubPoint::Start);
            facts.cfg_edge.push((start, mid));
            facts.cfg_edge.push((mid, next_start));
        }

        let term_idx = bb_data.statements.len();
        eprintln!("  {}: {:?}", term_idx, bb_data.terminator());
        let term_start = maps.point(bb, term_idx, SubPoint::Start);
        let term_mid = maps.point(bb, term_idx, SubPoint::Mid);
        facts.cfg_edge.push((term_start, term_mid));
        for succ in bb_data.terminator().successors() {
            let succ_start = maps.point(succ, 0, SubPoint::Start);
            facts.cfg_edge.push((term_mid, succ_start));
        }
    }

    // From rustc_borrowck::nll::populate_polonius_move_facts: "Non-arguments start out
    // deinitialised; we simulate this with an initial move".  On the other hand, arguments are
    // considered assigned at the entry point.
    let entry_point = maps.point(START_BLOCK, 0, SubPoint::Start);
    for local in mir.local_decls.indices() {
        if mir.local_kind(local) == LocalKind::Arg {
            let path = maps.path(
                &mut facts,
                Place {
                    local,
                    projection: List::empty(),
                },
            );
            facts.path_assigned_at_base.push((path, entry_point));
        } else {
            let path = maps.path(
                &mut facts,
                Place {
                    local,
                    projection: List::empty(),
                },
            );
            facts.path_moved_at_base.push((path, entry_point));
        }
    }

    // Populate `use_of_var_derefs_origin`, and generate `LTy`s for all locals.
    let ltcx = LabeledTyCtxt::new(tcx);
    let mut local_ltys = Vec::with_capacity(mir.local_decls.len());
    for local in mir.local_decls.indices() {
        let lty = assign_origins(
            ltcx,
            hypothesis,
            &mut facts,
            &mut maps,
            acx.local_tys[local],
        );
        let var = maps.variable(local);
        lty.for_each_label(&mut |label| {
            if let Some(origin) = label.origin {
                facts.use_of_var_derefs_origin.push((var, origin));
            }
        });
        local_ltys.push(lty);
    }

    let mut loans = HashMap::<Local, Vec<(Path, Loan, BorrowKind)>>::new();
    // Populate `loan_issued_at` and `loans`.
    type_check::visit(
        tcx,
        ltcx,
        &mut facts,
        &mut maps,
        &mut loans,
        &local_ltys,
        mir,
    );

    // Populate `loan_invalidated_at`
    def_use::visit_loan_invalidated_at(acx.tcx(), &mut facts, &mut maps, &loans, mir);

    // Populate `var_defined/used/dropped_at` and `path_assigned/accessed_at_base`.
    def_use::visit(&mut facts, &mut maps, mir);

    dump::dump_facts_to_dir(&facts, &maps, format!("inspect/{}", name)).unwrap();

    let output = polonius_engine::Output::compute(&facts, polonius_engine::Algorithm::Naive, true);
    dump::dump_output_to_dir(&output, &maps, format!("inspect/{}", name)).unwrap();

    (facts, maps, output)
}

fn assign_origins<'tcx>(
    ltcx: LTyCtxt<'tcx>,
    hypothesis: &PointerTableMut<PermissionSet>,
    _facts: &mut AllFacts,
    maps: &mut AtomMaps<'tcx>,
    lty: crate::LTy<'tcx>,
) -> LTy<'tcx> {
    ltcx.relabel(lty, &mut |lty| {
        let perm = if lty.label.is_none() {
            PermissionSet::empty()
        } else {
            hypothesis[lty.label]
        };
        match lty.ty.kind() {
            TyKind::Ref(_, _, _) | TyKind::RawPtr(_) => {
                let origin = Some(maps.origin());
                Label { origin, perm }
            }
            _ => Label { origin: None, perm },
        }
    })
}
