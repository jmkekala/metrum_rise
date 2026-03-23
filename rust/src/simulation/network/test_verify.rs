#[cfg(test)]
mod tests {
    use crate::simulation::network::graph::{verify_intersection_geometry};
    use godot::prelude::Vector3;

    #[test]
    fn test_verify_normal() {
        let mut poly_3d = Vec::new();
        let ct = Vector3::new(0.0, 0.0, 0.0);
        let mut add_triangle = |a: Vector3, mut b: Vector3, mut c: Vector3| {
            let normal = (b - a).cross(c - a);
            if normal.length() < 0.0001 { return; } 
            if normal.y < 0.0 {
                std::mem::swap(&mut b, &mut c); 
            }
            poly_3d.push(a); poly_3d.push(b); poly_3d.push(c);
        };
        add_triangle(ct, Vector3::new(1.0, 0.0, 1.0), Vector3::new(-1.0, 0.0, 1.0));
        match verify_intersection_geometry(ct, &poly_3d) {
            Ok(_) => println!("Pass!"),
            Err(e) => println!("Error: {}", e),
        }
    }
}
