def log_execution(func):
    def wrapper(*args, **kwargs):
        print(f"Executing {func.__name__}")
        result = func(*args, **kwargs)
        print(f"Executed {func.__name__}")
        return result
    return wrapper

@log_execution
def some_function():
    print("Function logic here")

some_function()
