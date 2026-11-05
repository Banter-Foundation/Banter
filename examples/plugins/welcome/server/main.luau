banter.storage.set("hello", "ready")

banter.events.on("message_created", function(event)
  if event.message.body == "test" then
    return
  end
  banter.messages.edit(event.message.id, "test")
end)
