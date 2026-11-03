use super::{AlignedGrid, MutableSubgrid, SharedSubgrid};

fn numbered(width: usize, height: usize) -> AlignedGrid<i32> {
    let mut grid = AlignedGrid::with_alloc_tracker(width, height, None).unwrap();
    for y in 0..height {
        for x in 0..width {
            *grid.get_mut(x, y) = (y * 100 + x) as i32;
        }
    }
    grid
}

#[test]
fn groups_cover_the_grid_disjointly() {
    let mut grid = numbered(10, 7);
    let groups = grid.as_subgrid_mut().into_groups(4, 3);
    assert_eq!(groups.len(), 3 * 3);
    let sizes: Vec<_> = groups.iter().map(|g| (g.width(), g.height())).collect();
    assert_eq!(sizes[0], (4, 3));
    assert_eq!(sizes[2], (2, 3));
    assert_eq!(sizes[8], (2, 1));
    assert_eq!(groups[4].get(1, 2), 505);
    for mut g in groups {
        for y in 0..g.height() {
            for v in g.get_row_mut(y) {
                *v += 1;
            }
        }
    }
    assert_eq!(grid.get(9, 6), 610);
    assert_eq!(grid.get(0, 0), 1);
}

#[test]
fn fixed_count_groups_out_of_bounds_are_empty() {
    let mut grid = numbered(5, 5);
    let groups = grid.as_subgrid_mut().into_groups_with_fixed_count(4, 4, 3, 3);
    assert_eq!(groups.len(), 9);
    assert_eq!((groups[1].width(), groups[1].height()), (1, 4));
    assert_eq!((groups[2].width(), groups[2].height()), (0, 0));
    assert_eq!((groups[8].width(), groups[8].height()), (0, 0));
}

#[test]
fn shrink_and_extend_restore_hidden_columns_and_rows() {
    let mut grid = numbered(6, 4);
    let mut residual = numbered(3, 4);
    let mut bottom = numbered(3, 2);
    let mut sub = grid.as_subgrid_mut();
    sub.shrink_width(3);
    sub.shrink_height(2);
    assert_eq!((sub.width(), sub.height()), (3, 2));
    assert!(sub.extend_height_from(&bottom.as_subgrid()));
    assert_eq!(sub.get(2, 3), 102);
    assert!(sub.extend_width_from(&residual.as_subgrid()));
    assert_eq!((sub.width(), sub.height()), (6, 4));
    assert_eq!(sub.get(5, 3), 302);
    // Nothing hidden is left: a further extension is refused.
    assert!(!sub.extend_width_from(&residual.as_subgrid_mut().as_shared()));
    let _ = bottom.as_subgrid_mut();
}

#[test]
fn out_of_range_access_is_none() {
    let mut grid = numbered(3, 2);
    let mut sub = grid.as_subgrid_mut();
    assert!(sub.try_get_ref(3, 0).is_none());
    assert!(sub.try_get_mut(0, 2).is_none());
    assert!(sub.try_get_row(2).is_none());
    sub.swap((0, 0), (5, 5));
    assert_eq!(sub.get(0, 0), 0);
    sub.swap((0, 0), (2, 1));
    assert_eq!((sub.get(0, 0), sub.get(2, 1)), (102, 0));
}

#[test]
fn subgrids_and_splits_are_clipped() {
    let mut grid = numbered(8, 8);
    let mut sub = grid.as_subgrid_mut();
    let part = sub.subgrid_mut(6..20, 7..);
    assert_eq!((part.width(), part.height()), (2, 1));
    let (left, right) = sub.split_horizontal(100);
    assert_eq!((left.width(), right.width()), (8, 0));
    let (top, bottom) = sub.split_vertical(3);
    assert_eq!((top.height(), bottom.height()), (3, 5));
    assert_eq!(bottom.get(1, 0), 301);

    let shared = grid.as_subgrid();
    let inner = shared.subgrid(2..5, 1..3);
    assert_eq!(inner.get(0, 0), 102);
    assert_eq!(inner.get_row(5), &[] as &[i32]);
}

#[test]
fn short_buffers_clip_instead_of_panicking() {
    let mut buf = vec![0i32; 10];
    let grid = MutableSubgrid::from_buf(&mut buf, 4, 5, 4);
    assert_eq!((grid.width(), grid.height()), (4, 2));
    let shared = SharedSubgrid::from_buf(&buf, 4, 5, 4);
    assert_eq!((shared.width(), shared.height()), (4, 2));
    assert!(AlignedGrid::<f32>::with_alloc_tracker(usize::MAX, 2, None).is_err());
}
