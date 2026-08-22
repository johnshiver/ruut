import pytest


from btree import BPlusTree, Internal, Leaf


def test_empty_tree():
    tree = BPlusTree()

    assert tree.get(10) is None
    assert tree.items() == []


def test_first_insert_creates_leaf_root():
    tree = BPlusTree()

    tree.insert(10, "A")

    assert isinstance(tree.root, Leaf)

    assert tree.root.keys == [10]
    assert tree.root.values == ["A"]

    assert tree.get(10) == "A"


def test_leaf_keeps_keys_sorted():
    tree = BPlusTree()

    tree.insert(20, "B")
    tree.insert(5, "A")
    tree.insert(10, "C")

    assert isinstance(tree.root, Leaf)

    assert tree.root.keys == [5, 10, 20]

    assert tree.items() == [
        (5, "A"),
        (10, "C"),
        (20, "B"),
    ]


def test_update_existing_key():
    tree = BPlusTree()

    tree.insert(10, "old")
    tree.insert(10, "new")

    assert tree.get(10) == "new"

    # No duplicate key.
    assert tree.items() == [
        (10, "new"),
    ]


def test_root_leaf_splits():
    tree = BPlusTree()

    tree.insert(5, "A")
    tree.insert(10, "B")
    tree.insert(20, "C")

    # MAX_KEYS = 3, so root is still one leaf.
    assert isinstance(tree.root, Leaf)

    tree.insert(30, "D")

    # Fourth key causes the root leaf to split.
    assert isinstance(tree.root, Internal)

    #              [20]
    #             /    \
    #        [5,10]   [20,30]

    assert tree.root.keys == [20]

    assert len(tree.root.children) == 2

    left = tree.root.children[0]
    right = tree.root.children[1]

    assert isinstance(left, Leaf)
    assert isinstance(right, Leaf)

    assert left.keys == [5, 10]
    assert right.keys == [20, 30]


def test_exact_separator_goes_right():
    tree = BPlusTree()

    node = Internal(
        keys=[5, 9],
        children=[
            Leaf(keys=[1]),
            Leaf(keys=[5]),
            Leaf(keys=[9]),
        ],
    )

    assert tree._find_child(node, 4) == 0

    # Exact 5 goes right of separator 5.
    assert tree._find_child(node, 5) == 1

    assert tree._find_child(node, 7) == 1

    # Exact 9 goes right of separator 9.
    assert tree._find_child(node, 9) == 2

    assert tree._find_child(node, 100) == 2


def test_lookup_after_root_split():
    tree = BPlusTree()

    tree.insert(5, "A")
    tree.insert(10, "B")
    tree.insert(20, "C")
    tree.insert(30, "D")

    assert tree.get(5) == "A"
    assert tree.get(10) == "B"
    assert tree.get(20) == "C"
    assert tree.get(30) == "D"

    assert tree.get(15) is None
    assert tree.get(999) is None


def test_multiple_leaf_splits():
    tree = BPlusTree()

    for i in range(1, 11):
        tree.insert(i, str(i))

    assert tree.items() == [
        (1, "1"),
        (2, "2"),
        (3, "3"),
        (4, "4"),
        (5, "5"),
        (6, "6"),
        (7, "7"),
        (8, "8"),
        (9, "9"),
        (10, "10"),
    ]

    for i in range(1, 11):
        assert tree.get(i) == str(i)


def test_insert_in_reverse_order():
    tree = BPlusTree()

    for i in range(10, 0, -1):
        tree.insert(i, str(i))

    assert [key for key, _ in tree.items()] == list(range(1, 11))


def test_many_inserts():
    tree = BPlusTree()

    for i in range(100):
        tree.insert(i, f"value-{i}")

    for i in range(100):
        assert tree.get(i) == f"value-{i}"

    assert len(tree.items()) == 100


def test_leaf_links_are_correct():
    tree = BPlusTree()

    for i in range(1, 11):
        tree.insert(i, str(i))

    # Find leftmost leaf.
    node = tree.root

    while isinstance(node, Internal):
        node = node.children[0]

    seen = []

    # Traverse using ONLY next pointers.
    while node is not None:
        seen.extend(node.keys)
        node = node.next

    assert seen == list(range(1, 11))
