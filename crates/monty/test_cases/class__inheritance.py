# === Single inheritance: method resolution ===
class Animal:
    kind = 'animal'

    def __init__(self, name):
        self.name = name

    def speak(self):
        return 'generic'

    def describe(self):
        return self.name + ' says ' + self.speak()


class Dog(Animal):
    def speak(self):
        return 'woof'


d = Dog('rex')
assert d.name == 'rex'
assert d.speak() == 'woof'
# An inherited method calling an overridden one dispatches on the instance.
assert d.describe() == 'rex says woof'
# Inherited class variables resolve through the base.
assert d.kind == 'animal'
assert Dog.kind == 'animal'


# === Inherited __init__ ===
class Puppy(Dog):
    pass


p = Puppy('bud')
assert p.name == 'bud'
assert p.speak() == 'woof'


# === Deeper chains ===
class Base:
    def where(self):
        return 'base'


class Middle(Base):
    pass


class Leaf(Middle):
    pass


assert Leaf().where() == 'base'

# === isinstance walks the chain ===
assert isinstance(p, Puppy)
assert isinstance(p, Dog)
assert isinstance(p, Animal)
assert not isinstance(Animal('x'), Dog)
assert isinstance(p, (Base, Animal))
assert not isinstance(p, (Base, Middle))

# === issubclass walks the chain ===
assert issubclass(Puppy, Dog)
assert issubclass(Puppy, Animal)
assert issubclass(Animal, Animal)
assert not issubclass(Animal, Dog)
assert issubclass(Leaf, (Animal, Base))

# === type() identity is the most derived class ===
assert type(p) is Puppy
assert type(p) is not Dog


# === Overriding a class variable ===
class Cat(Animal):
    kind = 'feline'


assert Cat('tom').kind == 'feline'
assert Animal('x').kind == 'animal'


# === super() ===
class Counter:
    def __init__(self, start):
        self.value = start

    def label(self):
        return 'counter'


class Doubling(Counter):
    def __init__(self, start):
        super().__init__(start * 2)

    def label(self):
        return 'doubling of ' + super().label()


doubled = Doubling(5)
assert doubled.value == 10
assert doubled.label() == 'doubling of counter'


# super() from a middle class resolves against that class, not the receiver's.
class Three(Doubling):
    pass


assert Three(4).value == 8
assert Three(4).label() == 'doubling of counter'

# === The 3-arg type() constructor takes bases too ===
Made = type('Made', (Animal,), {'speak': lambda self: 'made'})
made = Made('m')
assert made.speak() == 'made'
assert made.describe() == 'm says made'
assert isinstance(made, Animal)
