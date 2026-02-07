# Monty-NEAR Example Contract
#
# Exercises every available NEAR host function:
#   value_return, input, log, storage_write, storage_read,
#   storage_remove, storage_has_key, current_account_id,
#   predecessor_account_id, signer_account_id, block_height,
#   block_timestamp, sha256, keccak256


def hello():
    """Return a greeting. Exercises: value_return."""
    value_return("Hello from Monty on NEAR!")


def echo():
    """Return the raw input back to the caller. Exercises: input, value_return."""
    data = input()
    value_return(data)


def greet():
    """
    Read a name from input and return a personalised greeting.
    Expects raw string input (not JSON).
    Exercises: input, value_return, string formatting.
    """
    name = input()
    if name == "":
        name = "World"
    value_return("Hello, " + name + "!")


def counter():
    """
    Increment a persistent counter and return the new value.
    Exercises: storage_read, storage_write, value_return.
    """
    count = storage_read("count")
    if count is None:
        count = 0
    else:
        count = int(count)
    count = count + 1
    storage_write("count", str(count))
    value_return(str(count))


def get_counter():
    """
    Read the current counter without modifying it.
    Exercises: storage_read, value_return.
    """
    count = storage_read("count")
    if count is None:
        value_return("0")
    else:
        value_return(count)


def set_get():
    """
    Write the input to storage under 'mykey' and read it back.
    Exercises: input, storage_write, storage_read, value_return.
    """
    data = input()
    storage_write("mykey", data)
    result = storage_read("mykey")
    value_return(result)


def remove_key():
    """
    Remove 'mykey' from storage and confirm it's gone.
    Exercises: storage_remove, storage_has_key, value_return.
    """
    storage_remove("mykey")
    exists = storage_has_key("mykey")
    if exists:
        value_return("still exists")
    else:
        value_return("removed")


def whoami():
    """
    Return the contract's own account ID and current block height.
    Exercises: current_account_id, block_height, value_return.
    """
    account = current_account_id()
    height = block_height()
    value_return(account + " at block " + str(height))


def caller_info():
    """
    Return info about who called this method and when.
    Exercises: predecessor_account_id, signer_account_id,
               block_height, block_timestamp, value_return.
    """
    predecessor = predecessor_account_id()
    signer = signer_account_id()
    height = block_height()
    timestamp = block_timestamp()
    result = "predecessor=" + predecessor
    result = result + " signer=" + signer
    result = result + " block=" + str(height)
    result = result + " timestamp=" + str(timestamp)
    value_return(result)


def hash_it():
    """
    Compute SHA-256 and Keccak-256 of the input, return both.
    Exercises: input, sha256, keccak256, value_return.
    """
    data = input()
    s = sha256(data)
    k = keccak256(data)
    value_return("sha256=" + s + " keccak256=" + k)


def log_and_return():
    """
    Log a message and return a confirmation.
    Exercises: input, log, value_return.
    """
    msg = input()
    if msg == "":
        msg = "default log message"
    log("LOG: " + msg)
    value_return("logged: " + msg)


def kv_put():
    """
    Generic key-value store: write key=<first line>, value=<second line>.
    Input format: "key:value" separated by a colon.
    Exercises: input, storage_write, value_return.
    """
    data = input()
    pos = data.find(":")
    if pos < 0:
        value_return("error: expected key:value")
    else:
        key = data[0:pos]
        val = data[pos + 1 :]
        storage_write(key, val)
        value_return("ok")


def kv_get():
    """
    Generic key-value store: read by key.
    Exercises: input, storage_read, value_return.
    """
    key = input()
    val = storage_read(key)
    if val is None:
        value_return("")
    else:
        value_return(val)
