%% Ten million calls of one gen_server, one after the other.
-module(ask).
-behaviour(gen_server).
-export([main/0, init/1, handle_call/3, handle_cast/2]).

init(N) -> {ok, N}.
handle_call({add, By}, _From, N) -> {reply, N + By, N + By}.
handle_cast(_, N) -> {noreply, N}.

loop(_Pid, 0, Last) -> Last;
loop(Pid, I, _Last) -> loop(Pid, I - 1, gen_server:call(Pid, {add, 1})).

main() ->
    {ok, Pid} = gen_server:start(?MODULE, 0, []),
    Last = loop(Pid, 10000000, 0),
    io:format("~p~n", [Last]),
    halt().
