#!/usr/bin/env python
"""Convert demo notebooks to markdown with frontmatter extracted from first cell."""

import shutil
import nbconvert
import json
import yaml
from pathlib import Path
from typing import Dict, Optional


def extract_frontmatter_from_cell(cell_source: str) -> Optional[Dict]:
    """
    Extract title and description from first cell.
    Expected format:
    # Title Here
    Description text here
    Can be multiple lines
    """
    lines = cell_source.strip().split('\n')

    if not lines:
        return None

    title = None
    description_lines = []

    for line in lines:
        line = line.strip()
        if line.startswith('# '):
            title = line[2:].strip()
        elif line:  # Non-empty line
            description_lines.append(line)

    if not title:
        return None

    description = ' '.join(description_lines).strip()

    return {
        'title': title,
        'description': description if description else title
    }


def get_frontmatter_from_notebook(notebook_path: Path) -> Optional[str]:
    """Extract frontmatter from the first cell of a notebook."""
    try:
        with open(notebook_path, 'r', encoding='utf-8') as f:
            notebook = json.load(f)

        if not notebook.get('cells'):
            return None

        first_cell = notebook['cells'][0]

        # Only process markdown cells
        if first_cell.get('cell_type') != 'markdown':
            return None

        cell_source = ''.join(first_cell.get('source', []))

        frontmatter_dict = extract_frontmatter_from_cell(cell_source)

        if frontmatter_dict:
            # Use yaml.dump for proper formatting
            yaml_content = yaml.dump(
                frontmatter_dict,
                default_flow_style=False,
                allow_unicode=True,
                sort_keys=False
            )
            return f"---\n{yaml_content}---"

        return None
    except Exception as e:
        print(f"Warning: Could not extract frontmatter from {notebook_path}: {e}")
        return None


def convert_notebook(
    notebook_path: Path,
    output_path: Path,
    frontmatter: Optional[str] = None,
    skip_first_cell: bool = True
) -> None:
    """Convert notebook to markdown with optional frontmatter."""
    exporter = nbconvert.MarkdownExporter()
    body, _ = exporter.from_filename(str(notebook_path))

    if skip_first_cell:
        # Remove first markdown cell from output
        lines = body.split('\n')

        # Skip lines until we find the end of first cell
        start_idx = 0
        found_first_heading = False

        for i, line in enumerate(lines):
            if line.startswith('# ') and not found_first_heading:
                found_first_heading = True
                continue
            elif found_first_heading and (line.startswith('#') or line.startswith('```')):
                start_idx = i
                break
            elif found_first_heading and line.strip() == '':
                continue
            elif found_first_heading and not line.startswith('#'):
                if line.strip():
                    start_idx = i
                    break

        if start_idx > 0:
            body = '\n'.join(lines[start_idx:]).lstrip('\n')

    # Prepare output
    output_path.parent.mkdir(parents=True, exist_ok=True)

    if frontmatter:
        content = f"{frontmatter}\n{body}"
    else:
        content = body

    output_path.write_text(content, encoding='utf-8')
    print(f"✓ Converted {notebook_path.name} -> {output_path}")


def main():
    """Convert all notebooks in demos/ to markdown."""

    # Get the repo root (parent of site/)
    repo_root = Path(__file__).parent.parent

    # Map source directories to destination directories (relative to repo root)
    # Format: {source_path: destination_path}
    DIRS = {
        "capsem_python/demos/tutorials": "site/src/content/docs/tutorials",
        "capsem_python/demos/policies": "site/src/content/docs/policies",
        "capsem_proxy/demo": "site/src/content/docs/proxy"
    }

    for src_dir, dest_dir in DIRS.items():

        demos_dir = repo_root / src_dir
        output_dir = repo_root / dest_dir

        # Skip if source directory doesn't exist
        if not demos_dir.exists():
            print(f"⚠ Skipping {src_dir} (not found)")
            continue

        # Clean destination
        shutil.rmtree(output_dir, ignore_errors=True)
        output_dir.mkdir(parents=True, exist_ok=True)

        # Find all notebooks
        notebooks = list(demos_dir.glob("*.ipynb"))

        if not notebooks:
            print(f"⚠ No notebooks found in {demos_dir}")
            continue

        print(f"Found {len(notebooks)} notebook(s) in {demos_dir} to convert:\n")

        for notebook_path in sorted(notebooks):
            # Get frontmatter from first cell
            frontmatter = get_frontmatter_from_notebook(notebook_path)

            if not frontmatter:
                print(f"⚠ No frontmatter found in {notebook_path.name}, using defaults")
                title = notebook_path.stem.replace('_', ' ').title()
                frontmatter = f"---\ntitle: {title}\ndescription: {title} guide\n---"

            # Determine output filename (notebook_name.md)
            output_filename = notebook_path.stem + ".md"
            output_path = output_dir / output_filename

            # Convert with frontmatter, skipping first cell
            convert_notebook(
                notebook_path,
                output_path,
                frontmatter=frontmatter,
                skip_first_cell=True
            )

        print(f"\n✓ All notebooks in {demos_dir} converted to {output_dir}")

    return 0


if __name__ == "__main__":
    exit(main())
