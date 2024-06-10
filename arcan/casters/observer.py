class Observer:
    def update(self, message: str):
        pass

class CasterObserver(Observer):
    def update(self, message: str):
        print(f"Caster Observer: {message}")

class Subject:
    def __init__(self):
        self._observers = []

    def attach(self, observer: Observer):
        self._observers.append(observer)

    def detach(self, observer: Observer):
        self._observers.remove(observer)

    def notify(self, message: str):
        for observer in self._observers:
            observer.update(message)

# Usage
subject = Subject()
observer = CasterObserver()
subject.attach(observer)
subject.notify("Caster state changed")
