# An instance of a class that inherits `str` is a string that owns a reference
# on its class, so the class is held by its name and by each live instance.


class Act(str):
    def who(self):
        return 'w'


a = Act('one')
b = Act('two')
gone = Act('three')
del gone
# ref-counts={'Act': 3, 'a': 1, 'b': 1}
