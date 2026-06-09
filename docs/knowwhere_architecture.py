"""KnowWhere Architecture - Clear, Understandable Visualization
Run: manim -qH knowwhere_architecture.py KnowWhereArchitecture
"""

from manim import *

config.background_color = "#0a0e1a"
config.output_file = "/Users/nimarfranklinmac/.hermes/videos/knowwhere_architecture"
config.format = "mp4"
config.quality = "high_quality"
config.fps = 30


class KnowWhereArchitecture(Scene):
    def construct(self):
        # === SCENE 1: The Problem ===
        self._show_problem()

        # === SCENE 2: The Solution Overview ===
        self._show_solution_overview()

        # === SCENE 3: Component Map ===
        self._show_component_map()

        # === SCENE 4: Query Flow (Main Animation) ===
        self._show_query_flow()

        # === SCENE 5: Memory Storage Flow ===
        self._show_storage_flow()

        # === SCENE 6: Pointer-First Architecture ===
        self._show_pointer_first()

        # === SCENE 7: DREAM Mode ===
        self._show_dream_mode()

    def _show_problem(self):
        """Show why we need KnowWhere."""
        title = Text("The Problem", font_size=48, color=RED_C, weight=BOLD)
        title.move_to(UP * 2.5)
        self.play(Write(title))

        problems = [
            "AI Agents forget context",
            "Vector DBs are just soup of embeddings",
            "No understanding of memory importance",
            "No automated cleanup or consolidation",
        ]

        items = VGroup(*[Text(f"✗ {p}", font_size=24, color=RED) for p in problems])
        items.arrange(DOWN, aligned_edge=LEFT, buff=0.4)
        items.move_to(DOWN * 0.5)

        for item in items:
            self.play(Write(item), run_time=0.3)

        self.wait(2)
        self.play(FadeOut(title), FadeOut(items))

    def _show_solution_overview(self):
        """KnowWhere is a fractal memory system."""
        title = Text("KnowWhere: Fractal Memory", font_size=44, color=GREEN_C, weight=BOLD)
        subtitle = Text("Memories organized by importance, automatically", font_size=22, color=GREY)

        title.move_to(UP * 2.5)
        subtitle.next_to(title, DOWN)

        # Visual: pyramid of importance
        pyramid = self._make_pyramid()
        pyramid.move_to(DOWN * 0.8)

        labels = VGroup(
            Text("L2: Important", font_size=14, color=YELLOW),
            Text("L1: Context", font_size=14, color=BLUE),
            Text("L0: Raw", font_size=14, color=GREY),
        )
        labels[0].move_to(pyramid.get_edge_center(UP) + UP * 0.3)
        labels[1].move_to(pyramid.get_edge_center(LEFT) + LEFT * 0.3)
        labels[2].move_to(pyramid.get_edge_center(DOWN) + DOWN * 0.3)

        self.play(Write(title))
        self.play(Write(subtitle))
        self.play(Create(pyramid))
        for label in labels:
            self.play(Write(label))

        self.wait(2)
        self.play(FadeOut(title), FadeOut(subtitle), FadeOut(pyramid), FadeOut(labels))

    def _make_pyramid(self):
        """Create a simple pyramid shape."""
        l2 = Polygon([-1.5, 0, 0], [1.5, 0, 0], [0.75, 1, 0], [-0.75, 1, 0],
                    fill_opacity=0.4, stroke_color=YELLOW, stroke_width=2)
        l1 = Polygon([-1.5, 0, 0], [1.5, 0, 0], [2, -1.5, 0], [-2, -1.5, 0],
                    fill_opacity=0.3, stroke_color=BLUE, stroke_width=2)
        l1.set_fill(BLUE, opacity=0.3)

        # L0 base
        l0 = Polygon([-2, -1.5, 0], [2, -1.5, 0], [2.5, -3, 0], [-2.5, -3, 0],
                    fill_opacity=0.2, stroke_color=GREY, stroke_width=2)
        l0.set_fill(GREY, opacity=0.2)

        return VGroup(l0, l1, l2)

    def _show_component_map(self):
        """Show the 4 main components."""
        title = Text("4 Core Components", font_size=40, color=WHITE, weight=BOLD)
        title.move_to(UP * 3.2)
        self.play(Write(title))

        # 4 boxes in a row
        components = [
            ("1. API", "User requests\nAuth & Rate\nLimits", BLUE),
            ("2. Memory", "Fractal Nodes\nTiered Storage\nDREAM Mode", GREEN),
            ("3. Storage", "PostgreSQL\n+ pgvector\n(JSON backup)", ORANGE),
            ("4. Ollama", "Embeddings\n(LOCAL!)\nNo API costs", PURPLE),
        ]

        boxes = VGroup()
        for i, (header, body, color) in enumerate(components):
            box = RoundedRectangle(width=2.2, height=2.5, corner_radius=0.15,
                                  fill_opacity=0.15, stroke_color=color, stroke_width=2)

            h_text = Text(header, font_size=18, color=color, weight=BOLD)
            b_text = Text(body, font_size=13, color=WHITE)

            header_box = VGroup(h_text)
            body_box = VGroup(*[Text(line, font_size=13, color=WHITE) for line in body.split('\n')])
            body_box.arrange(DOWN, buff=0.15)

            full = VGroup(box, h_text, body_box)
            h_text.move_to(box.get_top() + DOWN * 0.4)
            body_box.move_to(box.get_center() + DOWN * 0.3)

            full.move_to(LEFT * 4.5 + RIGHT * (i * 3))

            boxes.add(full)
            self.play(Create(box), Write(h_text), run_time=0.3)

        # Arrows between boxes
        for i in range(len(boxes) - 1):
            start = boxes[i].get_edge_center(RIGHT)
            end = boxes[i + 1].get_edge_center(LEFT)
            arrow = Arrow(start, end, buff=0.2, color=GREY, stroke_width=2)
            self.play(Create(arrow), run_time=0.2)

        self.wait(2)
        self.play(FadeOut(title), FadeOut(boxes))

    def _show_query_flow(self):
        """Main animation: show how a query flows through the system."""
        title = Text("How a Query Works", font_size=40, color=YELLOW_C, weight=BOLD)
        title.move_to(UP * 3.2)
        self.play(Write(title))

        # Step label
        step = Text("Step 1", font_size=16, color=GREY)
        step.move_to(UP * 2.5)

        # Create simplified components for flow
        user = self._make_component("User", "Who asks", GREY, LEFT * 4 + DOWN * 0.5, 1.5)
        api = self._make_component("API", "Auth check", BLUE, LEFT * 1.5 + DOWN * 0.5, 1.5)
        memory = self._make_component("Memory", "Find & rank", GREEN, RIGHT * 1.5 + DOWN * 0.5, 1.8)
        ollama = self._make_component("Ollama", "Embed query", PURPLE, RIGHT * 4 + DOWN * 0.5, 1.5)
        storage = self._make_component("Storage", "pgvector\nsearch", ORANGE, RIGHT * 4 + DOWN * 2.5, 1.3)

        components = VGroup(user, api, memory, ollama, storage)

        # Draw arrows
        arrows = {
            "user_api": Arrow(user.get_right(), api.get_left(), buff=0.1, color=GREY),
            "api_mem": Arrow(api.get_right(), memory.get_left(), buff=0.1, color=BLUE),
            "mem_oll": Arrow(memory.get_right(), ollama.get_left(), buff=0.1, color=GREEN),
            "mem_stor": Arrow(memory.get_bottom(), storage.get_top(), buff=0.1, color=GREEN),
        }

        for arrow in arrows.values():
            self.play(Create(arrow), run_time=0.2)

        self.play(Create(user), Create(api), Create(memory), Create(ollama), Create(storage))

        # Animate query packet
        query = self._make_packet("Query:\n'Find my notes\nfrom yesterday'", YELLOW)
        query.move_to(user.get_left() + LEFT * 0.5)
        self.play(FadeIn(query))

        # Step 1: User -> API
        self.play(query.animate.move_to(api.get_left() + LEFT * 0.3), run_time=0.5)
        step_text = Text("✓ Auth OK", font_size=14, color=GREEN)
        step_text.next_to(api, DOWN)
        self.play(Write(step_text))

        # Step 2: API -> Memory
        self.play(query.animate.move_to(memory.get_left() + LEFT * 0.3), run_time=0.5)

        # Step 3: Memory -> Ollama (for embedding)
        embed_query = self._make_packet("Embed\nQuery", PURPLE)
        embed_query.move_to(memory.get_right())
        self.play(FadeIn(embed_query))
        self.play(embed_query.animate.move_to(ollama.get_left() + LEFT * 0.3), run_time=0.5)

        # Ollama responds
        vector = self._make_packet("[0.2, 0.8, ...]", PURPLE)
        vector.move_to(ollama.get_left())
        self.play(FadeOut(embed_query), FadeIn(vector))
        self.play(vector.animate.move_to(memory.get_right()), run_time=0.5)

        # Step 4: Memory -> Storage (vector search)
        vs_query = self._make_packet("Vector\nSearch", ORANGE)
        vs_query.move_to(memory.get_bottom())
        self.play(FadeIn(vs_query))
        self.play(vs_query.animate.move_to(storage.get_top()), run_time=0.5)

        results = self._make_packet("Top 10\nResults", GREEN)
        results.move_to(storage.get_top())
        self.play(FadeOut(vs_query), FadeIn(results))
        self.play(results.animate.move_to(memory.get_bottom()), run_time=0.5)

        # Step 5: Memory ranks and returns
        self.play(FadeOut(vector))
        answer = self._make_packet("Ranked\nResults", GREEN)
        answer.move_to(memory.get_left())
        self.play(FadeIn(answer))
        self.play(answer.animate.move_to(api.get_right()), run_time=0.5)
        self.play(answer.animate.move_to(user.get_left()), run_time=0.5)

        self.play(FadeOut(answer))

        # Summary
        summary = Text("Vector search + Semantic ranking + Importance tier", font_size=16, color=YELLOW)
        summary.move_to(DOWN * 2.8)
        self.play(Write(summary))

        self.wait(2)
        self.play(FadeOut(title), FadeOut(components), FadeOut(VGroup(*arrows.values())),
                  FadeOut(step_text), FadeOut(summary))

    def _make_component(self, header, body, color, position, width=1.5):
        """Create a component box."""
        box = RoundedRectangle(width=width, height=1.2, corner_radius=0.1,
                              fill_opacity=0.2, stroke_color=color, stroke_width=2)
        box.move_to(position)

        h = Text(header, font_size=16, color=color, weight=BOLD)
        h.move_to(box.get_top() + DOWN * 0.3)

        b_lines = body.split('\n')
        b = VGroup(*[Text(line, font_size=11, color=WHITE) for line in b_lines])
        b.arrange(DOWN, buff=0.05)
        b.move_to(box.get_center() + DOWN * 0.1)

        return VGroup(box, h, b)

    def _make_packet(self, text, color):
        """Create a data packet."""
        packet = RoundedRectangle(width=1.0, height=0.6, corner_radius=0.1,
                                  fill_opacity=0.9, stroke_color=color, stroke_width=2)
        packet.set_fill(color, opacity=0.8)
        t = Text(text, font_size=9, color=WHITE)
        t.move_to(packet.get_center())
        return VGroup(packet, t)

    def _show_storage_flow(self):
        """Show how a memory is stored."""
        title = Text("Storing a Memory", font_size=40, color=GREEN_C, weight=BOLD)
        title.move_to(UP * 3.2)
        self.play(Write(title))

        # Input
        input_box = RoundedRectangle(width=2, height=1.2, corner_radius=0.1,
                                    fill_opacity=0.3, stroke_color=YELLOW, stroke_width=2)
        input_text = Text("'Meeting notes\nfrom yesterday'", font_size=14, color=YELLOW)
        input_text.move_to(input_box.get_center())
        input_box.move_to(LEFT * 3 + DOWN * 0.5)

        self.play(Create(input_box), Write(input_text))

        # Arrow
        arrow = Arrow(input_box.get_right(), LEFT * 0.5 + DOWN * 0.5, buff=0.1, color=GREY)
        self.play(Create(arrow))

        # Process
        process_box = RoundedRectangle(width=2.5, height=1.5, corner_radius=0.1,
                                      fill_opacity=0.2, stroke_color=GREEN, stroke_width=2)
        process_texts = VGroup(
            Text("1. Generate embedding", font_size=12, color=WHITE),
            Text("2. Calculate importance", font_size=12, color=WHITE),
            Text("3. Store in correct tier", font_size=12, color=WHITE),
        )
        process_texts.arrange(DOWN, buff=0.1)
        process_box.move_to(LEFT * 0.5 + DOWN * 0.5)
        process_texts.move_to(process_box.get_center())

        self.play(Create(process_box), Write(process_texts))

        # Tiers
        tier_y = DOWN * 0.5
        tiers = VGroup()
        for i, (tier, color, desc) in enumerate([
            ("L0", GREY, "Raw memory"),
            ("L1", BLUE, "Semantic cluster"),
            ("L2", YELLOW, "Important"),
        ]):
            box = RoundedRectangle(width=1.5, height=0.8, corner_radius=0.1,
                                  fill_opacity=0.2, stroke_color=color, stroke_width=2)
            t = Text(f"{tier}\n{desc}", font_size=11, color=color)
            t.move_to(box.get_center())
            g = VGroup(box, t)
            g.move_to(RIGHT * 3 + RIGHT * (i * 2) + tier_y)
            tiers.add(g)

            self.play(Create(box), Write(t))

        # Arrow to tiers
        out_arrow = Arrow(process_box.get_right(), tiers.get_left(), buff=0.1, color=GREY)
        self.play(Create(out_arrow))

        self.wait(2)
        self.play(FadeOut(title), FadeOut(input_box), FadeOut(input_text),
                  FadeOut(arrow), FadeOut(process_box), FadeOut(process_texts),
                  FadeOut(out_arrow), FadeOut(tiers))

    def _show_pointer_first(self):
        """Explain Pointer-First architecture."""
        title = Text("Pointer-First Architecture", font_size=40, color=BLUE_C, weight=BOLD)
        title.move_to(UP * 3.2)
        self.play(Write(title))

        subtitle = Text("Never lose a memory — even when files move", font_size=18, color=GREY)
        subtitle.next_to(title, DOWN)
        self.play(Write(subtitle))

        # Show pointer chain
        # Memory ID -> Pointer -> File URI -> Actual File
        memory_id = self._make_data_box("Memory ID\nkw_abc123", BLUE, LEFT * 3.5 + DOWN * 0.5)
        pointer = self._make_data_box("Pointer\nfile:///data/...", GREEN, LEFT * 0.5 + DOWN * 0.5)
        file_uri = self._make_data_box("File\nnotes.txt", ORANGE, RIGHT * 2.5 + DOWN * 0.5)

        boxes = VGroup(memory_id, pointer, file_uri)
        for box in boxes:
            self.play(Create(box), run_time=0.3)

        # Arrows with labels
        arrow1 = Arrow(memory_id.get_right(), pointer.get_left(), buff=0.1, color=GREY)
        label1 = Text("Points to", font_size=12, color=GREY)
        label1.next_to(arrow1, DOWN, buff=0.1)

        arrow2 = Arrow(pointer.get_right(), file_uri.get_left(), buff=0.1, color=GREY)
        label2 = Text("Resolves to", font_size=12, color=GREY)
        label2.next_to(arrow2, DOWN, buff=0.1)

        self.play(Create(arrow1), Write(label1))
        self.play(Create(arrow2), Write(label2))

        # Benefit
        benefit = Text("✓ Self-Healing: If file moves, pointer redirects automatically",
                      font_size=16, color=GREEN)
        benefit.move_to(DOWN * 2.5)
        self.play(Write(benefit))

        self.wait(2)
        self.play(FadeOut(title), FadeOut(subtitle), FadeOut(boxes),
                  FadeOut(arrow1), FadeOut(arrow2), FadeOut(label1), FadeOut(label2),
                  FadeOut(benefit))

    def _make_data_box(self, text, color, position):
        box = RoundedRectangle(width=2.2, height=1.0, corner_radius=0.1,
                              fill_opacity=0.3, stroke_color=color, stroke_width=2)
        t = Text(text, font_size=14, color=color)
        box.move_to(position)
        t.move_to(position)
        return VGroup(box, t)

    def _show_dream_mode(self):
        """Explain DREAM Mode."""
        title = Text("DREAM Mode", font_size=44, color=GREEN_C, weight=BOLD)
        subtitle = Text("Automated memory maintenance while you sleep", font_size=18, color=GREY)

        title.move_to(UP * 3.0)
        subtitle.next_to(title, DOWN)

        self.play(Write(title), Write(subtitle))

        # 4 phases
        phases = [
            ("1. Audit", "Check energy\ndecay", BLUE, LEFT * 3.5),
            ("2. Dedup", "Merge\nduplicates", PURPLE, LEFT * 1.2),
            ("3. Consolidate", "L2→L1→L0\ncompaction", ORANGE, RIGHT * 1.2),
            ("4. Abstract", "Summarize\nwith VLM", GREEN, RIGHT * 3.5),
        ]

        phase_objects = VGroup()
        for i, (name, desc, color, pos) in enumerate(phases):
            box = RoundedRectangle(width=2, height=1.5, corner_radius=0.15,
                                  fill_opacity=0.2, stroke_color=color, stroke_width=2)

            name_t = Text(name, font_size=18, color=color, weight=BOLD)
            desc_t = Text(desc, font_size=12, color=WHITE)
            desc_t.arrange(DOWN, buff=0.05)

            group = VGroup(box, name_t, desc_t)
            name_t.move_to(box.get_top() + DOWN * 0.4)
            desc_t.move_to(box.get_center())
            group.move_to(pos + DOWN * 1.0)

            phase_objects.add(group)
            self.play(Create(box), Write(name_t), run_time=0.3)

            if i < len(phases) - 1:
                next_pos = phases[i + 1][3] + LEFT * 1
                arrow = Arrow(box.get_right(), next_pos + RIGHT * 1, buff=0.1, color=GREY)
                self.play(Create(arrow), run_time=0.2)

        # Schedule
        schedule = Text("Runs every 1 hour (configurable)", font_size=14, color=YELLOW)
        schedule.move_to(DOWN * 2.8)
        self.play(Write(schedule))

        self.wait(3)

    def _fade_out_all(self):
        """Clear the scene."""
        if self.mobjects:
            self.play(*[FadeOut(m) for m in self.mobjects], run_time=0.3)


if __name__ == "__main__":
    scene = KnowWhereArchitecture()
    scene.render()
