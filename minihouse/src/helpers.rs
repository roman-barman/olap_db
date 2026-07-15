pub fn assert_unique_names<T>(items: &[(String, T)]) {
    for i in 1..items.len() {
        let (name, _) = &items[i];
        assert!(
            !items[..i].iter().any(|(n, _)| n == name),
            "duplicate column name: {name}"
        );
    }
}
