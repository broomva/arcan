from abc import ABC, abstractmethod


class SpellCommand(ABC):
    @abstractmethod
    def execute(self):
        pass

class ScrappingSpell(SpellCommand):
    def execute(self):
        # Scrapping logic here
        pass

class SearchSpell(SpellCommand):
    def execute(self):
        # Search logic here
        pass

# Usage
def run_spell(spell: SpellCommand):
    spell.execute()

spell = ScrappingSpell()
run_spell(spell)
