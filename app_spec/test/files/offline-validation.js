const { one } = require('../config')

const delay = ms => new Promise(resolve => setTimeout(resolve, ms)) 

module.exports = scenario => {

      scenario('Can perform validation of an entry while the author is offline', async (s, t) => {
        
        const { alice, bob, carol } = await s.players({alice: one, bob: one, carol: one})
        await alice.spawn()
        await bob.spawn()

        // alice publishes the original entry. !This is an entry that requires full chain validation!
        const initialContent = "Holo world y'all"
        const params = { content: initialContent, in_reply_to: null }
        const create_result = await alice.call('app', "blog", "create_post", params)
        t.comment(JSON.stringify(create_result))
        t.equal(create_result.Ok.length, 46)

        t.comment('waiting for consistency between Alice and Bob')
        // bob will receive the entry and hold it
        await s.consistency()
        t.comment('consistency has been reached')
        
        // alice then goes offline
        t.comment('waiting for alice to go offline')
        await alice.kill()
        t.comment('alice has gone offline')

        // carol then comes online, will receive the entry via gossip from bob and need to validate
        // Since alice is offline the validation package cannot come direct and must
        // be regenerated from the published headers (which bob should hold)
        t.comment('waiting for Carol to come online')
        await carol.spawn()
        t.comment('Carol is online')

        t.comment('Waiting for Carol to get all data via gossip')
        await s.consistency()
        await delay(20000)
        t.comment('consistency has been reached')

        // Bob now go offline to ensure the following get_post uses carols local store only
        t.comment('waiting for Bob to go offline')
        await bob.kill()
        t.comment('Bob has gone offline')

        const post_address = create_result.Ok
        const params_get = { post_address }

        t.comment('Waiting for Carol to get post') // <- gets stuck here, times out
        const result = await carol.call('app', "blog", "get_post", params_get)
        t.comment(JSON.stringify(result))
        const value = JSON.parse(result.Ok.App[1])
        t.equal(value.content, initialContent)
      })

    }