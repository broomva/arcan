# includes the tools/spells that are self-targeted. Defines who the caster is

# Path: arcan/spells/self.py

#%%
# import os
# import subprocess
# from pathlib import Path


# def get_code(root_dir, output_md_path):
#     def write_and_append(file, lst, content):
#         try:
#             file.write(content)
#         except AttributeError as e:
#             print(e)
            
#         lst.append(content)
#     code = []
#     with open(output_md_path, 'w') as md_file:
#         # Adding tree command output to the markdown file
#         try:
#             tree_output = subprocess.check_output(['tree', root_dir], universal_newlines=True)
#             write_and_append('', code, f"\n## Directory Structure\n\n```\n{tree_output}\n```\n")
#         except FileNotFoundError:
#             write_and_append('',code,"\n## Directory Structure\n\n```\nTree command not available.\n```\n")
#         except subprocess.CalledProcessError as e:
#             write_and_append('',code,f"\n## Directory Structure\n\n```\nError executing tree command: {e}\n```\n")

#         for root, dirs, files in os.walk(root_dir):
#             for file in files:
#                 if file.endswith('.py'):
#                     file_path = Path(root) / file
#                     content_header = f"\n## {file_path}\n\n```python\n"
#                     write_and_append(md_file, code, content_header)
#                     with open(file_path, 'r') as py_file:
#                         file_content = py_file.read()
#                         write_and_append(md_file, code, file_content)
#                     content_footer = "\n```\n\n"
#                     write_and_append(md_file, code, content_footer)
#     return code


#%%



import fnmatch
import os
import re
import subprocess
from pathlib import Path


def read_gitignore(root_dir):
    ignore_patterns = []
    gitignore_path = Path(root_dir) / '.gitignore'
    if gitignore_path.exists():
        with open(gitignore_path, 'r') as f:
            ignore_patterns = [line.strip() for line in f.readlines() if line.strip() and not line.startswith('#')]
    return ignore_patterns

def should_ignore(file_path, ignore_patterns):
    return any(fnmatch.fnmatch(file_path, pattern) for pattern in ignore_patterns)

def remove_ansi_escape_codes(text):
    ansi_escape_pattern = re.compile(r'\x1B(?:[@-Z\\-_]|\[[0-?]*[ -/]*[@-~])')
    return ansi_escape_pattern.sub('', text)

def clean_output(text):
    # Remove ANSI escape codes
    ansi_escape_pattern = re.compile(r'\x1B(?:[@-Z\\-_]|\[[0-?]*[ -/]*[@-~])')
    cleaned_text = ansi_escape_pattern.sub('', text)
    # Replace non-breaking spaces with regular spaces
    cleaned_text = cleaned_text.replace('\xa0', ' ')
    return cleaned_text

def get_code(root_dir, output_md_path):
    def write_and_append(file, lst, content):
        try:
            file.write(content)
        except AttributeError as e:
            print(e)
        lst.append(content)

    ignore_patterns = read_gitignore(root_dir)
    # Convert ignore patterns suitable for the tree command
    tree_ignore_patterns = ','.join(ignore_patterns).replace('*', '') + ',*.pyc'
    code = []
    with open(output_md_path, 'w') as md_file:
        # Adding tree command output to the markdown file with filters
        try:
            tree_command = ['tree', root_dir, '-I', tree_ignore_patterns]
            tree_output = subprocess.check_output(tree_command, universal_newlines=True)
            cleaned_tree_output = clean_output(tree_output)
            write_and_append(md_file, code, f"\n## Directory Structure\n\n```\n{cleaned_tree_output}\n```\n")
        except FileNotFoundError:
            write_and_append(md_file, code, "\n## Directory Structure\n\n```\nTree command not available.\n```\n")
        except subprocess.CalledProcessError as e:
            write_and_append(md_file, code, f"\n## Directory Structure\n\n```\nError executing tree command: {e}\n```\n")


        for root, dirs, files in os.walk(root_dir):
            for file in files:
                file_path = Path(root) / file
                relative_file_path = file_path.relative_to(root_dir)
                if file.endswith('.py') and not file.endswith('.pyc') and not 'cpython' in file and not should_ignore(str(relative_file_path), ignore_patterns):
                    content_header = f"\n## {file_path}\n\n```python\n"
                    write_and_append(md_file, code, content_header)
                    with open(file_path, 'r') as py_file:
                        file_content = py_file.read()
                        write_and_append(md_file, code, file_content)
                    content_footer = "\n```\n\n"
                    write_and_append(md_file, code, content_footer)
    return code



def get_knowledge(caster):
    return [
        understanding for understanding in (
            [
                get_code(root_dir='../../', output_md_path='self_code.md'),
                # identity,
                # values,
                # beliefs,
                # desires,
                # intentions,
                # emotions,
                # thoughts,
                # memories,
                # experiences,
                # skills,
                # abilities,
                # powers,
                # strengths,
                # weaknesses,
                # limitations,
                # knowledge,
                # wisdom,
                # understanding,
                # intelligence,
                # intuition,
                # creativity,
                # imagination,
                # perception,
                # awareness,
                # consciousness,
                # subconsciousness,
                # unconsciousness,
                # self,
            ]
        )
    ]


def knowledge(caster):
    return get_knowledge(caster)


knowledge(1)
# %%
