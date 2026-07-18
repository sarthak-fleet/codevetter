use super::*;

pub fn shortest_path(
    snapshot: &StructuralGraphSnapshot,
    from: &str,
    to: &str,
    filter: &GraphQueryFilter,
) -> Result<GraphPathResult, String> {
    let start = resolve_node(snapshot, from)?;
    let target = resolve_node(snapshot, to)?;
    if start.id == target.id {
        return Ok(GraphPathResult {
            nodes: vec![start.clone()],
            edges: Vec::new(),
            total_cost: 0.0,
            visited: 1,
            truncated: false,
            context: query_context(snapshot),
        });
    }

    let mut adjacency: HashMap<&str, Vec<&StructuralGraphEdge>> = HashMap::new();
    let mut degree: HashMap<&str, usize> = HashMap::new();
    for edge in snapshot
        .edges
        .iter()
        .filter(|edge| edge_matches_filter(edge, filter))
    {
        adjacency.entry(&edge.from).or_default().push(edge);
        *degree.entry(&edge.from).or_default() += 1;
        *degree.entry(&edge.to).or_default() += 1;
    }
    for edges in adjacency.values_mut() {
        edges.sort_by(|left, right| left.id.cmp(&right.id));
    }

    let mut heap = BinaryHeap::new();
    heap.push(PathVisit::new(start.id.clone(), 0.0));
    let mut distance = HashMap::from([(start.id.clone(), 0.0)]);
    let mut previous: HashMap<String, (String, String)> = HashMap::new();
    let mut visited = 0;
    while let Some(current) = heap.pop() {
        if visited >= MAX_PATH_VISITS {
            break;
        }
        visited += 1;
        if current.node_id == target.id {
            break;
        }
        if current.cost > *distance.get(&current.node_id).unwrap_or(&f64::INFINITY) {
            continue;
        }
        for edge in adjacency
            .get(current.node_id.as_str())
            .into_iter()
            .flatten()
        {
            let hub_penalty = degree.get(edge.to.as_str()).copied().unwrap_or(0) as f64 * 0.002;
            let next_cost = current.cost + trust_cost(edge.trust) + hub_penalty;
            if next_cost < *distance.get(&edge.to).unwrap_or(&f64::INFINITY) {
                distance.insert(edge.to.clone(), next_cost);
                previous.insert(edge.to.clone(), (current.node_id.clone(), edge.id.clone()));
                heap.push(PathVisit::new(edge.to.clone(), next_cost));
            }
        }
    }

    let total_cost = distance.get(&target.id).copied().ok_or_else(|| {
        format!(
            "No directed graph path connects '{}' to '{}'",
            start.label, target.label
        )
    })?;
    let edge_by_id = snapshot
        .edges
        .iter()
        .map(|edge| (edge.id.as_str(), edge))
        .collect::<HashMap<_, _>>();
    let node_by_id = node_map(snapshot);
    let mut node_ids = vec![target.id.clone()];
    let mut edge_ids = Vec::new();
    let mut cursor = target.id.clone();
    while cursor != start.id {
        let (parent, edge_id) = previous
            .get(&cursor)
            .cloned()
            .ok_or_else(|| "Path reconstruction failed".to_string())?;
        node_ids.push(parent.clone());
        edge_ids.push(edge_id);
        cursor = parent;
    }
    node_ids.reverse();
    edge_ids.reverse();
    if edge_ids.len() > MAX_PATH_HOPS {
        return Err(format!(
            "No directed graph path within the {MAX_PATH_HOPS}-hop limit connects '{}' to '{}'",
            start.label, target.label
        ));
    }
    let mut result = GraphPathResult {
        nodes: node_ids
            .iter()
            .filter_map(|id| node_by_id.get(id.as_str()).copied().cloned())
            .collect(),
        edges: edge_ids
            .iter()
            .filter_map(|id| edge_by_id.get(id.as_str()).copied().cloned())
            .collect(),
        total_cost,
        visited,
        truncated: visited >= MAX_PATH_VISITS,
        context: query_context(snapshot),
    };
    enforce_path_bytes(&mut result)?;
    Ok(result)
}

pub fn impact(
    snapshot: &StructuralGraphSnapshot,
    reference: &str,
    direction: GraphDirection,
    depth: Option<usize>,
    filter: &GraphQueryFilter,
    limit: Option<usize>,
) -> Result<GraphImpactResult, String> {
    let root = resolve_node(snapshot, reference)?;
    let max_depth = depth.unwrap_or(3).clamp(1, 12);
    let limit = bounded_limit(limit);
    let mut adjacency: HashMap<&str, Vec<&StructuralGraphEdge>> = HashMap::new();
    let mut degree = HashMap::<&str, usize>::new();
    for edge in snapshot
        .edges
        .iter()
        .filter(|edge| edge_matches_filter(edge, filter))
    {
        *degree.entry(edge.from.as_str()).or_default() += 1;
        *degree.entry(edge.to.as_str()).or_default() += 1;
        match direction {
            GraphDirection::Incoming => adjacency.entry(&edge.to).or_default().push(edge),
            GraphDirection::Outgoing => adjacency.entry(&edge.from).or_default().push(edge),
            GraphDirection::Both => {
                adjacency.entry(&edge.to).or_default().push(edge);
                adjacency.entry(&edge.from).or_default().push(edge);
            }
        }
    }
    for (node_id, edges) in &mut adjacency {
        edges.sort_by(|left, right| {
            let left_neighbor = if left.from == *node_id {
                left.to.as_str()
            } else {
                left.from.as_str()
            };
            let right_neighbor = if right.from == *node_id {
                right.to.as_str()
            } else {
                right.from.as_str()
            };
            degree
                .get(left_neighbor)
                .copied()
                .unwrap_or_default()
                .cmp(&degree.get(right_neighbor).copied().unwrap_or_default())
                .then_with(|| left.id.cmp(&right.id))
        });
    }

    let mut queue = VecDeque::from([(root.id.as_str(), 0_usize)]);
    let mut seen = HashSet::from([root.id.as_str()]);
    let mut edge_ids = HashSet::new();
    let mut depth_reached = 0;
    let mut truncated = false;
    while let Some((node_id, current_depth)) = queue.pop_front() {
        depth_reached = depth_reached.max(current_depth);
        if current_depth >= max_depth {
            continue;
        }
        for edge in adjacency.get(node_id).into_iter().flatten() {
            edge_ids.insert(edge.id.as_str());
            let neighbor = if edge.from == node_id {
                edge.to.as_str()
            } else {
                edge.from.as_str()
            };
            if seen.insert(neighbor) {
                if seen.len() > limit + 1 {
                    truncated = true;
                    break;
                }
                queue.push_back((neighbor, current_depth + 1));
            }
        }
        if truncated {
            break;
        }
    }

    let node_by_id = node_map(snapshot);
    let mut affected = seen
        .into_iter()
        .filter(|id| *id != root.id)
        .filter_map(|id| node_by_id.get(id).copied().cloned())
        .collect::<Vec<_>>();
    affected.sort_by(|left, right| left.id.cmp(&right.id));
    affected.truncate(limit);
    let retained_ids = affected
        .iter()
        .map(|node| node.id.as_str())
        .chain(std::iter::once(root.id.as_str()))
        .collect::<HashSet<_>>();
    let mut edges = snapshot
        .edges
        .iter()
        .filter(|edge| edge_ids.contains(edge.id.as_str()))
        .filter(|edge| {
            retained_ids.contains(edge.from.as_str()) && retained_ids.contains(edge.to.as_str())
        })
        .take(MAX_EDGE_LIMIT)
        .cloned()
        .collect::<Vec<_>>();
    if edge_ids.len() > edges.len() {
        truncated = true;
    }
    edges.sort_by(|left, right| left.id.cmp(&right.id));
    let mut result = GraphImpactResult {
        root: root.clone(),
        affected,
        edges,
        depth_reached,
        truncated,
        context: query_context(snapshot),
    };
    enforce_impact_bytes(&mut result);
    Ok(result)
}
