from __future__ import annotations

from dataclasses import dataclass, field


MAX_KEYS = 3


# ---------------------------------------------------------------------------
# Nodes
# ---------------------------------------------------------------------------


@dataclass
class Leaf:
    keys: list[int] = field(default_factory=list)
    values: list[str] = field(default_factory=list)

    # Leaves form a linked list for efficient range scans.
    next: Leaf | None = None


@dataclass
class Internal:
    # These are separator keys, not actual records.
    #
    # Example:
    #
    #     keys = [5, 9]
    #
    #     children[0] -> keys < 5
    #     children[1] -> 5 <= keys < 9
    #     children[2] -> keys >= 9
    #
    keys: list[int] = field(default_factory=list)

    # Important invariant:
    #
    #     len(children) == len(keys) + 1
    #
    children: list[Node] = field(default_factory=list)


Node = Leaf | Internal


# ---------------------------------------------------------------------------
# B+ Tree
# ---------------------------------------------------------------------------


class BPlusTree:
    def __init__(self):
        self.root: Node | None = None

    # -----------------------------------------------------------------------
    # Lookup
    # -----------------------------------------------------------------------

    def get(self, key: int) -> str | None:
        """
        Look up a key.

        Internal nodes tell us which child to follow.
        Actual key/value data exists only in leaves.
        """
        node = self.root

        while node is not None:
            # We reached the actual data.
            if isinstance(node, Leaf):
                for i, existing_key in enumerate(node.keys):
                    if existing_key == key:
                        return node.values[i]

                return None

            # Otherwise this is an internal routing node.
            child_index = self._find_child(node, key)

            node = node.children[child_index]

        return None

    # -----------------------------------------------------------------------
    # Insert
    # -----------------------------------------------------------------------

    def insert(self, key: int, value: str) -> None:
        """
        Insert or update a key/value pair.
        """

        # Empty tree:
        #
        #     root
        #       |
        #       v
        #    [10:A]
        #
        if self.root is None:
            self.root = Leaf(
                keys=[key],
                values=[value],
            )
            return

        # Insert recursively.
        result = self._insert(self.root, key, value)

        # No split reached the root.
        if result is None:
            return

        # The old root split.
        #
        # result contains:
        #
        #     separator
        #     right sibling
        #
        separator, right = result

        old_root = self.root

        # Create a brand new internal root.
        #
        # Before:
        #
        #      [5 10 20 30]
        #
        # After:
        #
        #           [20]
        #          /    \
        #      [5 10]  [20 30]
        #
        self.root = Internal(
            keys=[separator],
            children=[
                old_root,
                right,
            ],
        )

    def _insert(
        self,
        node: Node,
        key: int,
        value: str,
    ) -> tuple[int, Node] | None:
        """
        Recursively insert into a subtree.

        Returns None if everything still fits.

        Returns:

            (separator_key, new_right_node)

        if the node had to split.

        The caller is responsible for inserting that separator/right-child
        into its own Internal node.
        """

        # -------------------------------------------------------------------
        # Leaf
        # -------------------------------------------------------------------

        if isinstance(node, Leaf):
            # Find sorted insertion position.
            i = 0

            while i < len(node.keys) and node.keys[i] < key:
                i += 1

            # Existing key: update value.
            if i < len(node.keys) and node.keys[i] == key:
                node.values[i] = value
                return None

            # New key.
            node.keys.insert(i, key)
            node.values.insert(i, value)

            # Leaf still fits.
            if len(node.keys) <= MAX_KEYS:
                return None

            # Leaf overflowed.
            return self._split_leaf(node)

        # -------------------------------------------------------------------
        # Internal node
        # -------------------------------------------------------------------

        child_index = self._find_child(node, key)

        child = node.children[child_index]

        result = self._insert(child, key, value)

        # Child did not split.
        if result is None:
            return None

        # Child split.
        separator, right_child = result

        # Suppose:
        #
        #     keys     = [5, 9]
        #     children = [C0, C1, C2]
        #
        # and C1 splits with separator 7.
        #
        # Then:
        #
        #     keys     = [5, 7, 9]
        #     children = [C0, C1-left, C1-right, C2]
        #
        node.keys.insert(child_index, separator)
        node.children.insert(child_index + 1, right_child)

        # Internal node still fits.
        if len(node.keys) <= MAX_KEYS:
            return None

        # Internal node also overflowed.
        return self._split_internal(node)

    # -----------------------------------------------------------------------
    # Leaf split
    # -----------------------------------------------------------------------

    def _split_leaf(
        self,
        leaf: Leaf,
    ) -> tuple[int, Leaf]:
        """
        Split an overflowing leaf.

        Example:

            [5, 10, 20, 30]

        becomes:

            [5, 10] -> [20, 30]

        The separator returned to the parent is 20.

        Notice that 20 remains in the leaf. This is a B+ tree:
        actual records remain at the leaf level.
        """

        mid = len(leaf.keys) // 2

        right = Leaf(
            keys=leaf.keys[mid:],
            values=leaf.values[mid:],
            next=leaf.next,
        )

        leaf.keys = leaf.keys[:mid]
        leaf.values = leaf.values[:mid]

        # Link the two leaves.
        leaf.next = right

        # First key of the right leaf becomes the separator.
        separator = right.keys[0]

        return separator, right

    # -----------------------------------------------------------------------
    # Internal split
    # -----------------------------------------------------------------------

    def _split_internal(
        self,
        node: Internal,
    ) -> tuple[int, Internal]:
        """
        Split an overflowing internal node.

        Unlike a leaf split, the middle separator MOVES UP into the parent.
        """

        mid = len(node.keys) // 2

        separator = node.keys[mid]

        right = Internal(
            keys=node.keys[mid + 1 :],
            children=node.children[mid + 1 :],
        )

        node.keys = node.keys[:mid]
        node.children = node.children[: mid + 1]

        return separator, right

    # -----------------------------------------------------------------------
    # Routing
    # -----------------------------------------------------------------------

    def _find_child(
        self,
        node: Internal,
        key: int,
    ) -> int:
        """
        Determine which child can contain key.

        Example:

            keys = [5, 9]

            children[0] -> key < 5
            children[1] -> 5 <= key < 9
            children[2] -> key >= 9

        Therefore:

            key = 3  -> child 0
            key = 5  -> child 1
            key = 7  -> child 1
            key = 9  -> child 2
            key = 15 -> child 2
        """

        i = 0

        # Exact matches go RIGHT.
        while i < len(node.keys) and key >= node.keys[i]:
            i += 1

        return i

    # -----------------------------------------------------------------------
    # Range / ordered iteration
    # -----------------------------------------------------------------------

    def items(self) -> list[tuple[int, str]]:
        """
        Return every key/value pair in ascending key order.

        B+ trees make this efficient because leaves are linked together.
        """

        if self.root is None:
            return []

        node = self.root

        # Descend to the leftmost leaf.
        while isinstance(node, Internal):
            node = node.children[0]

        result: list[tuple[int, str]] = []

        # Walk the linked leaf list.
        while node is not None:
            result.extend(zip(node.keys, node.values))
            node = node.next

        return result

    # -----------------------------------------------------------------------
    # Debug printing
    # -----------------------------------------------------------------------

    def print_tree(self) -> None:
        """
        Print the tree level by level.
        """

        if self.root is None:
            print("<empty>")
            return

        level: list[Node] = [self.root]

        while level:
            next_level: list[Node] = []

            parts = []

            for node in level:
                if isinstance(node, Leaf):
                    parts.append(f"Leaf{node.keys}")
                else:
                    parts.append(f"Internal{node.keys}")
                    next_level.extend(node.children)

            print(" | ".join(parts))

            level = next_level
