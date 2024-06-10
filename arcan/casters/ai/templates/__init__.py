def chains_templates():
    return {
        "chat_chain_template": """Consider the provided chat history and a subsequent question. Alternatively, conclude the conversation if it appears to be complete.

        Chat History:\"""
        {chat_history}
        \"""
        Follow Up Input: \"""
        {question}
        \"""
        Answer:""",
        "chat_prompt_template": """ 
        Question: {question}
        {context}
        Answer:""",
        "sql_chat_prompt_template": """ You're a senior SQL developer. You have to write sql code in snowflake database based on the following question. Also you have to ignore the sql keywords and give a one or two sentences about how did you arrive at that sql code. display the sql code in the code format (do not assume anything if the column is not available then say it is not available, do not make up code). Make sure the SQL code you create is a valid SQL ANSI code that works with pyspark dataframes

        Question: {question}
        {context}
        Answer:""",
        "sql_code_extraction_prompt": "Extract the input's text SQL query \n\n{text} \n\n. Only return the SQL code.",
        "validation_prompt": "You're a senior SQL and Machine Learning developer. Review the results provided and return feedback on the code and the answer:",
    }
