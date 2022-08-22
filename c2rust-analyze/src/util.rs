use rustc_hir::def::DefKind;
use rustc_middle::mir::{Local, Mutability, Operand, PlaceElem, PlaceRef, Rvalue};
use rustc_middle::ty::{DefIdTree, Ty, TyCtxt, TyKind};

#[derive(Debug)]
pub enum RvalueDesc<'tcx> {
    /// A pointer projection, such as `&(*x.y).z`.  The rvalue is split into a base pointer
    /// expression (in this case `x.y`) and a projection (`.z`).  The `&` and `*` are implicit.
    Project {
        /// The base pointer of the projection.  This is guaranteed to evaluate to a pointer or
        /// reference type.
        ///
        /// This may contain derefs, indicating that the pointer was loaded through another
        /// pointer.  Only the outermost deref is implicit.  For example, `&(**x).y` has a `base`
        /// of `*x` and a `proj` of `.y`.
        base: PlaceRef<'tcx>,
        /// The projection applied to the pointer.  This contains no `Deref` projections.
        proj: &'tcx [PlaceElem<'tcx>],
    },
    /// The address of a local or one of its fields, such as `&x.y`.  The rvalue is split into a
    /// base local (in this case `x`) and a projection (`.y`).  The `&` is implicit.
    AddrOfLocal {
        local: Local,
        /// The projection applied to the local.  This contains no `Deref` projections.
        proj: &'tcx [PlaceElem<'tcx>],
    },
}

pub fn describe_rvalue<'tcx>(rv: &Rvalue<'tcx>) -> Option<RvalueDesc<'tcx>> {
    Some(match *rv {
        Rvalue::Use(ref op) => match *op {
            Operand::Move(pl) | Operand::Copy(pl) => RvalueDesc::Project {
                base: pl.as_ref(),
                proj: &[],
            },
            Operand::Constant(_) => return None,
        },
        Rvalue::Ref(_, _, pl) | Rvalue::AddressOf(_, pl) => {
            let projection = &pl.projection[..];
            match projection
                .iter()
                .rposition(|p| matches!(p, PlaceElem::Deref))
            {
                Some(i) => {
                    // `i` is the index of the last `ProjectionElem::Deref` in `pl`.
                    RvalueDesc::Project {
                        base: PlaceRef {
                            local: pl.local,
                            projection: &projection[..i],
                        },
                        proj: &projection[i + 1..],
                    }
                }
                None => {
                    // `pl` refers to a field/element of a local.
                    RvalueDesc::AddrOfLocal {
                        local: pl.local,
                        proj: projection,
                    }
                }
            }
        }
        _ => return None,
    })
}

#[derive(Debug)]
pub enum Callee<'tcx> {
    PtrOffset {
        pointee_ty: Ty<'tcx>,
        mutbl: Mutability,
    },
}

pub fn ty_callee<'tcx>(tcx: TyCtxt<'tcx>, ty: Ty<'tcx>) -> Option<Callee<'tcx>> {
    let (did, _substs) = match *ty.kind() {
        TyKind::FnDef(did, substs) => (did, substs),
        _ => return None,
    };
    let name = tcx.item_name(did);

    match name.as_str() {
        "offset" => {
            // The `offset` inherent method of `*const T` and `*mut T`.
            let parent_did = tcx.parent(did);
            if tcx.def_kind(parent_did) != DefKind::Impl {
                return None;
            }
            if tcx.impl_trait_ref(parent_did).is_some() {
                return None;
            }
            let parent_impl_ty = tcx.type_of(parent_did);
            let (pointee_ty, mutbl) = match parent_impl_ty.kind() {
                TyKind::RawPtr(tm) => (tm.ty, tm.mutbl),
                _ => return None,
            };
            Some(Callee::PtrOffset { pointee_ty, mutbl })
        }
        _ => None,
    }
}
