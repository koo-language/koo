%% Sixteen gen_servers each count the primes in a range of their own, at the same time.
-module(parallel).
-behaviour(gen_server).
-export([main/0, init/1, handle_call/3, handle_cast/2]).

init(_) -> {ok, 0}.
handle_cast({count, From, To}, _) -> {noreply, count(From, To, 0)}.
handle_call(result, _From, Found) -> {reply, Found, Found}.

count(To, To, N) -> N;
count(I, To, N) ->
    case is_prime(I) of
        true -> count(I + 1, To, N + 1);
        false -> count(I + 1, To, N)
    end.

is_prime(N) when N < 2 -> false;
is_prime(N) -> is_prime(N, 2).

is_prime(N, D) when D * D > N -> true;
is_prime(N, D) when N rem D =:= 0 -> false;
is_prime(N, D) -> is_prime(N, D + 1).

main() ->
    Pids = [element(2, gen_server:start(?MODULE, [], [])) || _ <- lists:seq(1, 16)],
    lists:foreach(
        fun({I, Pid}) -> gen_server:cast(Pid, {count, I * 1000000, (I + 1) * 1000000}) end,
        lists:zip(lists:seq(0, 15), Pids)),
    Total = lists:sum([gen_server:call(Pid, result, infinity) || Pid <- Pids]),
    io:format("~p~n", [Total]),
    halt().
